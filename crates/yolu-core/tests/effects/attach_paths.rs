//! 層のパス: 描いた結果とパスは 1 回の Undo で入れ替わる。予算・検査・取消で断ったら何も変えない。パスの層に手で描かない。
use crate::attach_support;
use attach_support::*;
use std::sync::atomic::AtomicBool;
use yolu_core::geometry::{SurfaceGeometry, SurfaceTriangle, DEFAULT_WELD_TOLERANCE};
use yolu_core::glam::{Vec2, Vec3};
use yolu_core::paths::{
    fingerprint, render_canvas, render_surface, CanvasPath, CanvasPoint, ChannelPaint, Options,
    PathBrush, PathPoint, SurfacePath,
};
use yolu_core::{BrushSettings, Channel, CoreError, Document, LayerId, LayerPath, Rgba8, Surface};

fn options() -> Options<'static> {
    Options {
        width: W,
        height: H,
        tile_size: 8,
        ..Options::default()
    }
}

fn brush(radius: f64) -> PathBrush {
    PathBrush(BrushSettings {
        radius,
        spacing: 0.17,
        color: Rgba8::new(201, 37, 89, 219),
        ..BrushSettings::default()
    })
}

fn canvas(material: Option<Vec<ChannelPaint>>) -> CanvasPath {
    CanvasPath {
        id: 0x77,
        channel: Channel::Color,
        brush: brush(3.0),
        points: vec![
            CanvasPoint::new(4.5, 7.25, 0.3).unwrap(),
            CanvasPoint::new(29.5, 21.5, 0.9).unwrap(),
            CanvasPoint::new(37.25, 5.75, 0.55).unwrap(),
        ],
        material,
    }
}

fn pair() -> Vec<ChannelPaint> {
    vec![
        ChannelPaint {
            channel: Channel::Color,
            color: Rgba8::new(31, 87, 231, 255),
        },
        ChannelPaint {
            channel: Channel::Roughness,
            color: Rgba8::new(140, 140, 140, 255),
        },
    ]
}

fn plane() -> SurfaceGeometry {
    let (a, b, c, d) = (Vec3::ZERO, Vec3::X, Vec3::new(1.0, 1.0, 0.0), Vec3::Y);
    SurfaceGeometry::new(
        vec![
            SurfaceTriangle::new(a, b, c, Vec2::ZERO, Vec2::X, Vec2::ONE),
            SurfaceTriangle::new(a, c, d, Vec2::ZERO, Vec2::ONE, Vec2::Y),
        ],
        1,
        DEFAULT_WELD_TOLERANCE,
    )
    .unwrap()
}

fn surface_path(g: &SurfaceGeometry) -> SurfacePath {
    SurfacePath {
        id: 9,
        channel: Channel::Height,
        brush: brush(0.1),
        points: vec![
            PathPoint::new(0, 0.2, 0.3, 1.0).unwrap(),
            PathPoint::new(1, 0.4, 0.2, 1.0).unwrap(),
        ],
        model_fingerprint: fingerprint(g),
        material: None,
    }
}

fn rendered(path: &CanvasPath) -> Vec<(Channel, Surface)> {
    render_canvas(path, &options()).unwrap().channels
}

type LayerState = (Option<LayerPath>, Vec<(Channel, Vec<u8>)>, Vec<Channel>);

fn state(doc: &Document, id: LayerId) -> LayerState {
    let l = doc.layer(id).unwrap();
    (
        l.path().cloned(),
        l.surface_channels()
            .into_iter()
            .map(|c| (c, l.surface(c).unwrap().to_canvas_bytes()))
            .collect(),
        l.enabled_channels(),
    )
}

#[test]
fn a_path_and_its_pixels_swap_in_one_undo_step_and_enable_what_it_draws() {
    let (mut doc, l) = world();
    let layer = doc.add_layer("パス").unwrap();
    let before = state(&doc, layer);
    let steps = doc.undo_count();
    // 組（Color と Roughness）。Roughness は層で有効でない
    let path = canvas(Some(pair()));
    doc.set_canvas_path(layer, path.clone()).unwrap();
    assert_eq!(doc.undo_count(), steps + 1);
    let after = state(&doc, layer);
    assert!(
        doc.layer(layer)
            .unwrap()
            .is_channel_enabled(Channel::Roughness),
        "有効にする"
    );
    assert!(after
        .1
        .iter()
        .any(|(c, px)| *c == Channel::Roughness && px.iter().any(|b| *b != 0)));
    assert_ne!(before, after);
    doc.undo().unwrap();
    assert_eq!(state(&doc, layer), before, "パス・画素・有効が元に戻る");
    doc.redo().unwrap();
    assert_eq!(state(&doc, layer), after);
    let _ = l;
    // 組を Color だけに差し替えると、外れた Roughness は空になる。Undo で戻る
    let narrower = canvas(None);
    doc.set_canvas_path(layer, narrower).unwrap();
    assert!(
        doc.layer(layer)
            .unwrap()
            .surface(Channel::Roughness)
            .unwrap()
            .tile_count()
            == 0
    );
    doc.undo().unwrap();
    assert_eq!(state(&doc, layer), after);
    // 一度ラスタライズすると画素だけが残る
    doc.rasterize(layer).unwrap();
    assert!(doc.layer(layer).unwrap().path().is_none());
    assert_eq!(state(&doc, layer).1, after.1);
    doc.undo().unwrap();
    assert_eq!(state(&doc, layer), after);
}

#[test]
fn a_3d_path_is_set_with_its_rendered_surface() {
    let (mut doc, _) = world();
    let g = plane();
    let layer = doc.add_layer("面のパス").unwrap();
    doc.set_channel_enabled(layer, Channel::Height, true)
        .unwrap();
    let path = surface_path(&g);
    let drawn = render_surface(&path, &g, &options()).unwrap();
    assert!(drawn.dabs > 0);
    let steps = doc.undo_count();
    doc.set_path(layer, LayerPath::Surface(path.clone()), drawn.channels)
        .unwrap();
    assert_eq!(doc.undo_count(), steps + 1);
    assert!(matches!(
        doc.layer(layer).unwrap().path(),
        Some(LayerPath::Surface(_))
    ));
    // キャンバスのパスへは替えられない（種類を変えない）
    let other = canvas(None);
    assert!(doc
        .set_path(layer, LayerPath::Canvas(other.clone()), rendered(&other))
        .is_err());
    assert!(doc.layer(layer).unwrap().path().is_some());
}

#[test]
fn refusals_keep_the_layer_exactly_as_it_was() {
    let (mut doc, l) = world();
    let layer = doc.add_layer("パス").unwrap();
    let path = canvas(None);
    let drawn = rendered(&path);
    let reference = state(&doc, layer);
    let steps = doc.undo_count();
    let revision = doc.revision();
    let check = |doc: &mut Document, what: &str, r: Result<(), CoreError>| {
        assert!(r.is_err(), "{what}");
        assert_eq!(state(doc, layer), reference, "{what}");
        assert_eq!(doc.undo_count(), steps, "{what}");
        assert_eq!(doc.revision(), revision, "{what}");
    };
    // 面が足りない・多い・重なる・大きさが違う
    let r = doc.set_path(layer, LayerPath::Canvas(path.clone()), vec![]);
    check(&mut doc, "面が無い", r);
    let mut twice = drawn.clone();
    twice.push((Channel::Color, drawn[0].1.clone()));
    let r = doc.set_path(layer, LayerPath::Canvas(path.clone()), twice);
    check(&mut doc, "面が多い", r);
    let small = render_canvas(
        &path,
        &Options {
            width: 16,
            height: 16,
            tile_size: 8,
            ..Options::default()
        },
    )
    .unwrap()
    .channels;
    let r = doc.set_path(layer, LayerPath::Canvas(path.clone()), small);
    check(&mut doc, "面の大きさが違う", r);
    // 組のチャンネルの面だけを渡す
    let material = canvas(Some(pair()));
    let r = doc.set_path(layer, LayerPath::Canvas(material), drawn.clone());
    check(&mut doc, "組のチャンネルが足りない", r);
    // パスの検査（点・ブラシ・標準のチャンネル）
    let mut bad = path.clone();
    bad.brush = brush(0.0);
    let r = doc.set_path(layer, LayerPath::Canvas(bad), drawn.clone());
    check(&mut doc, "ブラシの半径", r);
    // 層の種類・チャンネル
    let r = doc.set_path(l[3], LayerPath::Canvas(path.clone()), drawn.clone());
    assert!(r.is_err(), "塗りつぶしの層");
    let mut disabled = canvas(None);
    disabled.channel = Channel::Metallic;
    let r = doc.set_path(
        layer,
        LayerPath::Canvas(disabled.clone()),
        rendered(&disabled),
    );
    check(&mut doc, "有効でないチャンネル（組なし）", r);
    // 予算: 巻き戻しの写し・層の画素
    doc.set_stroke_budget_bytes(1).unwrap();
    let r = doc.set_path(layer, LayerPath::Canvas(path.clone()), drawn.clone());
    assert_eq!(r, Err(CoreError::StrokeBudgetExceeded));
    assert_eq!(state(&doc, layer), reference);
    doc.set_stroke_budget_bytes(64 * 1024 * 1024).unwrap();
    // 画素の予算（今の画素ちょうど）では、1 タイルも増やせない
    let used = doc.allocated_bytes();
    doc.set_source_budget_bytes(used).unwrap();
    let r = doc.set_path(layer, LayerPath::Canvas(path.clone()), drawn.clone());
    assert_eq!(r, Err(CoreError::SourceBudgetExceeded));
    assert_eq!(state(&doc, layer), reference);
    assert_eq!(doc.undo_count(), steps);
    doc.set_source_budget_bytes(256 * 1024 * 1024).unwrap();
    doc.set_path(layer, LayerPath::Canvas(path), drawn).unwrap();
}

#[test]
fn a_path_layer_refuses_hand_painting_until_it_is_rasterized() {
    let (mut doc, _) = world();
    let layer = doc.add_layer("パス").unwrap();
    doc.set_canvas_path(layer, canvas(None)).unwrap();
    let brush = BrushSettings::default();
    assert!(doc.begin_stroke_in(layer, Channel::Color, &brush).is_err());
    assert!(doc
        .fill(
            layer,
            Channel::Color,
            Rgba8::new(1, 2, 3, 255),
            1.0,
            None,
            false
        )
        .is_err());
    assert!(doc
        .begin_material_stroke(
            layer,
            &[yolu_core::material::ChannelPaint::new(
                Channel::Color,
                Rgba8::new(1, 2, 3, 255)
            )],
            &brush
        )
        .is_err());
    assert!(doc
        .set_channel_enabled(layer, Channel::Color, false)
        .is_err());
    // マスクはパスの画素でないので描ける
    doc.add_layer_mask(layer).unwrap();
    doc.fill_mask(layer, 1.0, None, false).unwrap();
    doc.rasterize(layer).unwrap();
    assert!(doc
        .begin_stroke_in(layer, Channel::Color, &brush)
        .map(|s| doc.cancel_stroke(s))
        .is_ok());
    doc.set_channel_enabled(layer, Channel::Color, false)
        .unwrap();
}

#[test]
fn adding_a_path_layer_is_one_undo_step_and_a_refusal_adds_no_layer() {
    let (mut doc, l) = world();
    let path = canvas(Some(pair()));
    let drawn = rendered(&path);
    let count = doc.layers().len();
    let steps = doc.undo_count();
    let id = doc
        .add_path_layer(
            "新しい",
            LayerPath::Canvas(path.clone()),
            drawn.clone(),
            Some(l[0]),
        )
        .unwrap();
    assert_eq!(doc.layers().len(), count + 1);
    assert_eq!(doc.undo_count(), steps + 1);
    assert_eq!(
        doc.layer_index(id).unwrap(),
        doc.layer_index(l[0]).unwrap() + 1
    );
    assert!(doc
        .layer(id)
        .unwrap()
        .is_channel_enabled(Channel::Roughness));
    doc.undo().unwrap();
    assert!(doc.layer(id).is_none());
    doc.redo().unwrap();
    assert!(doc.layer(id).is_some());
    doc.undo().unwrap();
    // 画素の予算に収まらなければ、層も作らない
    doc.set_source_budget_bytes(doc.allocated_bytes()).unwrap();
    assert!(doc
        .add_path_layer("入らない", LayerPath::Canvas(path), drawn, None)
        .is_err());
    assert_eq!(doc.layers().len(), count);
    assert_eq!(doc.undo_count(), steps);
}

#[test]
fn cancelling_the_render_changes_nothing() {
    let (mut doc, _) = world();
    let layer = doc.add_layer("パス").unwrap();
    let before = state(&doc, layer);
    let cancel = AtomicBool::new(true);
    let r = doc.set_canvas_path_cancellable(layer, canvas(None), Some(&cancel));
    assert_eq!(r, Err(CoreError::Cancelled));
    assert_eq!(state(&doc, layer), before);
}

#[test]
fn duplicating_a_path_layer_renews_ids_and_keeps_the_path() {
    let (mut doc, _) = world();
    let layer = doc.add_layer("パス").unwrap();
    doc.set_canvas_path(layer, canvas(None)).unwrap();
    let copy = doc.duplicate_layer(layer, None).unwrap();
    let (a, b) = (
        doc.layer(layer).unwrap().path().unwrap(),
        doc.layer(copy).unwrap().path().unwrap(),
    );
    assert_ne!(a.id(), b.id(), "パスの ID は新しい");
    match (a, b) {
        (LayerPath::Canvas(a), LayerPath::Canvas(b)) => {
            assert_eq!(
                (&a.points, a.brush, a.channel),
                (&b.points, b.brush, b.channel)
            )
        }
        _ => panic!("同じ種類"),
    }
}
