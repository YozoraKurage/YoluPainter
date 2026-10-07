//! 層のパスの一覧の保存・復元（正本の版 27）。2 本以上・名前を付けた・隠したパスの層がある文書だけが版 27 になり、名前の無い見せる 1 本の
//! パスは今の欄のまま（前の版・同じバイト列）。往復・版の選び方・読み手の拒否（0.4.x の読み手の範囲・版の数だけの書き換え）・.ylsmart の断りを試す。
use yolu_core::geometry::{SurfaceGeometry, SurfaceTriangle, DEFAULT_WELD_TOLERANCE};
use yolu_core::glam::DVec2;
use yolu_core::glam::{Vec2, Vec3};
use yolu_core::paths::{
    fingerprint, render_list, CanvasPath, CanvasPoint, LayerPathEntry, Options, PathBrush,
    PathKind, PathPoint, Ribbon, RibbonMode, SurfacePath, Tangent,
};
use yolu_core::{BrushSettings, Channel, Document, LayerId, LayerPath, Rgba8};
use yolu_io::smart::{SmartFile, REFUSAL_PATH_LISTS};
use yolu_io::{
    NativeDocument, NativeValue, WriterInfo, MAX_NATIVE_VERSION, PATHS_VERSION,
    UNITY_NATIVE_VERSION,
};

fn writer() -> WriterInfo {
    WriterInfo {
        app: "YoluPainter-rs".into(),
        version: "0.0.1".into(),
        unity: "none".into(),
    }
}

fn canvas(id: u128, y: f64, color: Rgba8) -> CanvasPath {
    CanvasPath {
        style: Default::default(),
        id,
        channel: Channel::Color,
        brush: PathBrush(BrushSettings {
            radius: 2.5,
            color,
            ..BrushSettings::default()
        }),
        points: vec![
            CanvasPoint::new(3.5, y, 0.7).unwrap(),
            CanvasPoint::new(28.5, y + 3.0, 1.0).unwrap(),
        ],
        material: None,
    }
}

fn entry(p: CanvasPath, name: &str, visible: bool) -> LayerPathEntry {
    LayerPathEntry {
        name: name.into(),
        visible,
        path: LayerPath::Canvas(p),
    }
}

fn doc_with(entries: Vec<LayerPathEntry>) -> (Document, LayerId) {
    let mut doc = Document::with_tile_size(32, 32, 16).unwrap();
    let layer = doc.add_layer("パス").unwrap();
    doc.set_canvas_paths(layer, entries).unwrap();
    (doc, layer)
}

fn paths_of(doc: &Document) -> Vec<Vec<LayerPathEntry>> {
    doc.layers().iter().map(|l| l.paths().to_vec()).collect()
}

fn pixels_of(doc: &Document) -> Vec<Vec<u8>> {
    doc.layers()
        .iter()
        .flat_map(|l| {
            l.surface_channels()
                .into_iter()
                .map(|c| l.surface(c).unwrap().to_canvas_bytes())
        })
        .collect()
}

#[test]
fn only_a_list_layer_makes_a_version_27_document() {
    assert_eq!(PATHS_VERSION, 27);
    // 名前の無い見せる 1 本は今の欄のまま、Unity 版と同じ版
    let (one, _) = doc_with(vec![entry(
        canvas(1, 6.5, Rgba8::new(200, 10, 10, 255)),
        "",
        true,
    )]);
    let native = NativeDocument::from_core(&one).unwrap();
    assert_eq!(native.version(), UNITY_NATIVE_VERSION);
    assert!(native.fields().iter().all(|f| !f.path.contains(".paths.")));
    assert!(native
        .fields()
        .iter()
        .any(|f| f.path.ends_with(".canvas_path.point_count")));
    // 2 本・名前を付けた 1 本・隠した 1 本は版 27 の一覧
    for entries in [
        vec![
            entry(canvas(1, 6.5, Rgba8::new(200, 10, 10, 255)), "", true),
            entry(canvas(2, 16.5, Rgba8::new(10, 10, 200, 255)), "", true),
        ],
        vec![entry(
            canvas(1, 6.5, Rgba8::new(200, 10, 10, 255)),
            "縁",
            true,
        )],
        vec![entry(
            canvas(1, 6.5, Rgba8::new(200, 10, 10, 255)),
            "",
            false,
        )],
    ] {
        let (doc, _) = doc_with(entries.clone());
        let native = NativeDocument::from_core(&doc).unwrap();
        assert_eq!(native.version(), PATHS_VERSION);
        assert_eq!(
            native.field("layers[0].paths.count"),
            Some(&NativeValue::Int(entries.len() as i32))
        );
        assert_eq!(
            native.field("layers[0].has_canvas_path"),
            Some(&NativeValue::Bool(false))
        );
        let back = native.to_core().unwrap();
        assert_eq!(paths_of(&back), paths_of(&doc));
        assert_eq!(pixels_of(&back), pixels_of(&doc));
        // 書き直しても、読み直しても同じバイト
        assert_eq!(
            NativeDocument::from_core(&back).unwrap().to_bytes(),
            native.to_bytes()
        );
    }
}

#[test]
fn a_list_of_3d_paths_round_trips() {
    let (a, b, c, d) = (Vec3::ZERO, Vec3::X, Vec3::new(1.0, 1.0, 0.0), Vec3::Y);
    let g = SurfaceGeometry::new(
        vec![
            SurfaceTriangle::new(a, b, c, Vec2::ZERO, Vec2::X, Vec2::ONE),
            SurfaceTriangle::new(a, c, d, Vec2::ZERO, Vec2::ONE, Vec2::Y),
        ],
        1,
        DEFAULT_WELD_TOLERANCE,
    )
    .unwrap();
    let path = |id: u128, v: f64| SurfacePath {
        style: Default::default(),
        id,
        channel: Channel::Color,
        brush: PathBrush(BrushSettings {
            radius: 0.06,
            ..BrushSettings::default()
        }),
        points: vec![
            PathPoint::new(0, 0.2, v, 1.0).unwrap(),
            PathPoint::new(1, 0.4, v, 0.5).unwrap(),
        ],
        model_fingerprint: fingerprint(&g),
        material: None,
    };
    let entries = vec![
        LayerPathEntry {
            name: "上".into(),
            visible: true,
            path: LayerPath::Surface(path(5, 0.1)),
        },
        LayerPathEntry {
            name: "下".into(),
            visible: false,
            path: LayerPath::Surface(path(6, 0.3)),
        },
    ];
    let mut doc = Document::with_tile_size(32, 32, 16).unwrap();
    let layer = doc.add_layer("面").unwrap();
    let drawn = render_list(
        &entries,
        Some(&g),
        &Options {
            width: 32,
            height: 32,
            tile_size: 16,
            ..Options::default()
        },
    )
    .unwrap();
    doc.set_paths(layer, entries.clone(), drawn.channels)
        .unwrap();
    let native = NativeDocument::from_core(&doc).unwrap();
    assert_eq!(native.version(), PATHS_VERSION);
    assert_eq!(
        native.field("layers[0].paths.items[1].surface"),
        Some(&NativeValue::Bool(true))
    );
    let back = native.to_core().unwrap();
    assert_eq!(back.layers()[0].paths(), &entries[..]);
    assert_eq!(pixels_of(&back), pixels_of(&doc));
}

#[test]
fn the_reader_refuses_lists_the_writer_never_writes() {
    let (doc, _) = doc_with(vec![
        entry(canvas(1, 6.5, Rgba8::new(200, 10, 10, 255)), "a", true),
        entry(canvas(2, 16.5, Rgba8::new(10, 10, 200, 255)), "b", true),
    ]);
    let native = NativeDocument::from_core(&doc).unwrap();
    // 名前の制御文字
    assert!(native
        .with_value(
            "layers[0].paths.items[0].name",
            NativeValue::Text("a\u{7}".into())
        )
        .is_err());
    // 同じ ID の 2 本は core が断る
    let same = native
        .with_value(
            "layers[0].paths.items[1].path.id",
            native
                .field("layers[0].paths.items[0].path.id")
                .unwrap()
                .clone(),
        )
        .unwrap();
    assert!(same.to_core().is_err());
    // 一覧と 1 本のパスの欄の両方は持てない
    assert!(native
        .with_value("layers[0].has_canvas_path", NativeValue::Bool(true))
        .is_err());
}

#[test]
fn older_readers_refuse_a_version_27_document_by_its_number() {
    let (doc, _) = doc_with(vec![
        entry(canvas(1, 6.5, Rgba8::new(200, 10, 10, 255)), "", true),
        entry(canvas(2, 16.5, Rgba8::new(10, 10, 200, 255)), "", true),
    ]);
    let native = NativeDocument::from_core(&doc).unwrap();
    let bytes = native.to_bytes();
    let version = i32::from_le_bytes(bytes[8..12].try_into().unwrap());
    // 0.4.x の読み手の範囲は 1〜26（26 は分けた正本）。27 はその外なので、版の数で断る
    assert_eq!(version, 27);
    assert!(!(1..=26).contains(&version));
    // 版の数だけを 25 に書き換えると、属性のビット 6 が知らないビットで読めない（版 25 の意味は変えない）
    let mut older = bytes.clone();
    older[8..12].copy_from_slice(&25i32.to_le_bytes());
    let err = NativeDocument::read(&older).unwrap_err();
    assert!(err.to_string().contains("属性"), "{err}");
    // この読み手より新しい版は読まない
    let mut newer = bytes;
    newer[8..12].copy_from_slice(&(MAX_NATIVE_VERSION + 1).to_le_bytes());
    assert!(NativeDocument::read(&newer).is_err());
}

#[test]
fn a_path_list_is_not_put_on_the_shelf_but_a_single_path_is() {
    let (doc, layer) = doc_with(vec![
        entry(canvas(1, 6.5, Rgba8::new(200, 10, 10, 255)), "", true),
        entry(canvas(2, 16.5, Rgba8::new(10, 10, 200, 255)), "", true),
    ]);
    let material = doc.capture_smart_material(&[layer], "素材").unwrap();
    let err = SmartFile::from_core(&material, &writer()).unwrap_err();
    assert!(err.to_string().contains(REFUSAL_PATH_LISTS), "{err}");
    let (doc, layer) = doc_with(vec![entry(
        canvas(1, 6.5, Rgba8::new(200, 10, 10, 255)),
        "",
        true,
    )]);
    let material = doc.capture_smart_material(&[layer], "素材").unwrap();
    assert!(SmartFile::from_core(&material, &writer()).is_ok());
}

#[test]
fn corners_and_handles_make_a_version_27_list_and_round_trip() {
    let mut p = canvas(1, 6.5, Rgba8::new(200, 10, 10, 255));
    p.points.push(CanvasPoint::new(20.5, 25.0, 0.4).unwrap());
    p.points[1].tangent = Tangent::Corner;
    p.points[2].tangent = Tangent::Handles {
        incoming: DVec2::new(-3.25, 1.5),
        outgoing: DVec2::new(0.0, -7.125),
    };
    let (doc, _) = doc_with(vec![entry(p.clone(), "", true)]);
    let native = NativeDocument::from_core(&doc).unwrap();
    assert_eq!(native.version(), PATHS_VERSION, "滑らかでない点は一覧の形");
    assert_eq!(
        native.field("layers[0].paths.items[0].extra.tangent_count"),
        Some(&NativeValue::Int(2))
    );
    assert_eq!(
        native.field("layers[0].paths.items[0].extra.tangents[1].index"),
        Some(&NativeValue::Int(2))
    );
    let back = native.to_core().unwrap();
    assert_eq!(paths_of(&back), paths_of(&doc));
    assert_eq!(pixels_of(&back), pixels_of(&doc));
    assert_eq!(
        NativeDocument::from_core(&back).unwrap().to_bytes(),
        native.to_bytes()
    );
    // 点を滑らかに戻すと、前の版の 1 本の欄に戻る
    let mut smooth = p;
    for q in &mut smooth.points {
        q.tangent = Tangent::Smooth;
    }
    let (doc, _) = doc_with(vec![entry(smooth, "", true)]);
    assert_eq!(
        NativeDocument::from_core(&doc).unwrap().version(),
        UNITY_NATIVE_VERSION
    );
}

#[test]
fn surface_handles_round_trip_exactly() {
    let (a, b, c, d) = (Vec3::ZERO, Vec3::X, Vec3::new(1.0, 1.0, 0.0), Vec3::Y);
    let g = SurfaceGeometry::new(
        vec![
            SurfaceTriangle::new(a, b, c, Vec2::ZERO, Vec2::X, Vec2::ONE),
            SurfaceTriangle::new(a, c, d, Vec2::ZERO, Vec2::ONE, Vec2::Y),
        ],
        1,
        DEFAULT_WELD_TOLERANCE,
    )
    .unwrap();
    let path = SurfacePath {
        style: Default::default(),
        id: 9,
        channel: Channel::Color,
        brush: PathBrush(BrushSettings {
            radius: 0.06,
            ..BrushSettings::default()
        }),
        points: vec![
            PathPoint::new(0, 0.2, 0.1, 1.0)
                .unwrap()
                .with_tangent(Tangent::Handles {
                    incoming: Vec3::new(0.1, -0.2, 0.3),
                    outgoing: Vec3::new(1.0 / 3.0, 0.0, -1e-7),
                }),
            PathPoint::new(1, 0.4, 0.3, 0.5)
                .unwrap()
                .with_tangent(Tangent::Corner),
        ],
        model_fingerprint: fingerprint(&g),
        material: None,
    };
    let entries = vec![LayerPathEntry::new(LayerPath::Surface(path))];
    let mut doc = Document::with_tile_size(32, 32, 16).unwrap();
    let layer = doc.add_layer("面").unwrap();
    let drawn = render_list(
        &entries,
        Some(&g),
        &Options {
            width: 32,
            height: 32,
            tile_size: 16,
            ..Options::default()
        },
    )
    .unwrap();
    doc.set_paths(layer, entries.clone(), drawn.channels)
        .unwrap();
    let native = NativeDocument::from_core(&doc).unwrap();
    assert_eq!(native.version(), PATHS_VERSION);
    assert_eq!(
        native.field("layers[0].paths.items[0].extra.tangents[0].outgoing.z"),
        Some(&NativeValue::Float(-1e-7f32 as f64))
    );
    let back = native.to_core().unwrap();
    assert_eq!(
        back.layers()[0].paths(),
        &entries[..],
        "f32 の取っ手がそのまま戻る"
    );
}

#[test]
fn the_reader_refuses_broken_tangents() {
    let mut p = canvas(1, 6.5, Rgba8::new(200, 10, 10, 255));
    p.points.push(CanvasPoint::new(20.5, 25.0, 0.4).unwrap());
    p.points[0].tangent = Tangent::Corner;
    p.points[2].tangent = Tangent::Corner;
    let (doc, _) = doc_with(vec![entry(p, "", true)]);
    let native = NativeDocument::from_core(&doc).unwrap();
    let at = |k: &str| format!("layers[0].paths.items[0].extra.{k}");
    // 点の番号が範囲外・増える順でない・知らない種類
    assert!(native
        .with_value(&at("tangents[1].index"), NativeValue::Int(3))
        .is_err());
    assert!(native
        .with_value(&at("tangents[1].index"), NativeValue::Int(0))
        .is_err());
    assert!(native
        .with_value(&at("tangents[0].kind"), NativeValue::Byte(3))
        .is_err());
}

fn with_kind(mut p: CanvasPath, kind: PathKind) -> CanvasPath {
    p.style.kind = kind;
    p
}

#[test]
fn every_kind_round_trips_and_only_a_plain_erase_stays_in_the_old_field() {
    let image = yolu_core::ImageId(0x5151);
    let new_doc = || {
        let mut doc = Document::with_tile_size(32, 32, 16).unwrap();
        doc.set_effect_inputs(
            yolu_core::EffectInputs::new().with_image(
                image,
                yolu_core::ImageInput::new(
                    2,
                    1,
                    vec![255, 0, 0, 255, 0, 0, 255, 255],
                    yolu_core::ImageColorSpace::Srgb,
                )
                .unwrap(),
            ),
        )
        .unwrap();
        doc
    };
    let kinds = [
        PathKind::Fill,
        PathKind::Smudge { strength: 0.25 },
        PathKind::Ribbon(Ribbon {
            image,
            mode: RibbonMode::Stretch,
            spacing: 1.5,
        }),
    ];
    for kind in kinds {
        let mut d = new_doc();
        let layer = d.add_layer("パス").unwrap();
        d.set_canvas_paths(
            layer,
            vec![entry(
                with_kind(canvas(1, 6.5, Rgba8::new(200, 10, 10, 255)), kind),
                "",
                true,
            )],
        )
        .unwrap();
        let native = NativeDocument::from_core(&d).unwrap();
        assert_eq!(native.version(), PATHS_VERSION, "{kind:?} は一覧の形");
        let back = native.to_core().unwrap();
        assert_eq!(paths_of(&back), paths_of(&d), "{kind:?}");
        assert_eq!(pixels_of(&back), pixels_of(&d));
    }
    // 消しゴムだけの 1 本は前の版の欄（ブラシの消しゴムの印）で書き、消しゴムの種類として読む
    let mut d = new_doc();
    let layer = d.add_layer("パス").unwrap();
    let erase = with_kind(
        canvas(1, 6.5, Rgba8::new(200, 10, 10, 255)),
        PathKind::Erase,
    );
    d.set_canvas_paths(layer, vec![entry(erase.clone(), "", true)])
        .unwrap();
    let native = NativeDocument::from_core(&d).unwrap();
    assert_eq!(native.version(), UNITY_NATIVE_VERSION);
    assert_eq!(
        native.field("layers[0].canvas_path.brush.erase"),
        Some(&NativeValue::Bool(true))
    );
    let back = native.to_core().unwrap();
    assert_eq!(paths_of(&back), paths_of(&d));
    // ブラシの印で消す古いパス（Unity 版の文書）も、消しゴムの種類として読み、同じバイトで書き直す
    let mut legacy = erase;
    legacy.style.kind = PathKind::Stroke;
    legacy.brush.0.erase = true;
    let mut d = new_doc();
    let layer = d.add_layer("パス").unwrap();
    d.set_canvas_paths(layer, vec![entry(legacy, "", true)])
        .unwrap();
    let native = NativeDocument::from_core(&d).unwrap();
    let back = native.to_core().unwrap();
    match back.layers()[0].path() {
        Some(LayerPath::Canvas(c)) => {
            assert_eq!(c.style.kind, PathKind::Erase);
            assert!(!c.brush.0.erase);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(
        NativeDocument::from_core(&back).unwrap().to_bytes(),
        native.to_bytes()
    );
}

#[test]
fn the_reader_refuses_unknown_kinds_and_ribbon_values() {
    let (doc, _) = doc_with(vec![entry(
        with_kind(canvas(1, 6.5, Rgba8::new(200, 10, 10, 255)), PathKind::Fill),
        "",
        true,
    )]);
    let native = NativeDocument::from_core(&doc).unwrap();
    assert!(native
        .with_value("layers[0].paths.items[0].extra.kind", NativeValue::Byte(5))
        .is_err());
    let image = yolu_core::ImageId(0x77);
    let mut d = Document::with_tile_size(32, 32, 16).unwrap();
    d.set_effect_inputs(
        yolu_core::EffectInputs::new().with_image(
            image,
            yolu_core::ImageInput::new(1, 1, vec![9, 9, 9, 255], yolu_core::ImageColorSpace::Srgb)
                .unwrap(),
        ),
    )
    .unwrap();
    let layer = d.add_layer("パス").unwrap();
    d.set_canvas_paths(
        layer,
        vec![entry(
            with_kind(
                canvas(1, 6.5, Rgba8::new(200, 10, 10, 255)),
                PathKind::Ribbon(Ribbon {
                    image,
                    mode: RibbonMode::Tile,
                    spacing: 1.0,
                }),
            ),
            "",
            true,
        )],
    )
    .unwrap();
    let native = NativeDocument::from_core(&d).unwrap();
    let at = |k: &str| format!("layers[0].paths.items[0].extra.ribbon.{k}");
    assert!(native
        .with_value(&at("spacing"), NativeValue::Float(5.0))
        .is_err());
    assert!(native
        .with_value(&at("mode"), NativeValue::Byte(2))
        .is_err());
    assert!(native
        .with_value(&at("image"), NativeValue::Guid([0; 16]))
        .is_err());
}

#[test]
fn a_tip_angle_and_depth_round_trip_in_the_list() {
    let mut p = canvas(1, 6.5, Rgba8::new(200, 10, 10, 255));
    p.style.tip = Some(std::sync::Arc::new(
        yolu_core::BrushTip::new("筆先", 3, 2, vec![0, 64, 128, 192, 255, 7]).unwrap(),
    ));
    p.style.angle = -37.5;
    p.style.follow = true;
    p.style.depth = Some(2.5);
    let (doc, _) = doc_with(vec![entry(p, "", true)]);
    let native = NativeDocument::from_core(&doc).unwrap();
    assert_eq!(
        native.version(),
        PATHS_VERSION,
        "筆先・角度・深さは一覧の形"
    );
    assert_eq!(
        native.field("layers[0].paths.items[0].extra.tip.width"),
        Some(&NativeValue::Int(3))
    );
    let back = native.to_core().unwrap();
    assert_eq!(paths_of(&back), paths_of(&doc));
    assert_eq!(pixels_of(&back), pixels_of(&doc));
    assert_eq!(
        NativeDocument::from_core(&back).unwrap().to_bytes(),
        native.to_bytes()
    );
    // 範囲の外の角度・深さは読み手が断る
    for (k, v) in [
        ("angle", NativeValue::Float(400.0)),
        ("depth", NativeValue::Float(100.0)),
    ] {
        assert!(native
            .with_value(&format!("layers[0].paths.items[0].extra.{k}"), v)
            .is_err());
    }
}

#[test]
fn symmetry_round_trips_and_the_reader_checks_its_side() {
    let mut p = canvas(1, 6.5, Rgba8::new(200, 10, 10, 255));
    p.style.symmetry = yolu_core::paths::PathSymmetry::Canvas(
        yolu_core::CanvasSymmetry::new(
            yolu_core::SymmetryMode::Radial,
            yolu_core::glam::DVec2::new(16.0, 15.5),
            5,
        )
        .unwrap(),
    );
    let (doc, _) = doc_with(vec![entry(p, "", true)]);
    let native = NativeDocument::from_core(&doc).unwrap();
    assert_eq!(native.version(), PATHS_VERSION);
    let back = native.to_core().unwrap();
    assert_eq!(paths_of(&back), paths_of(&doc));
    assert_eq!(pixels_of(&back), pixels_of(&doc));
    // 2D のパスに鏡の面の対称は書けない（読み手が断る）
    assert!(native
        .with_value(
            "layers[0].paths.items[0].extra.symmetry",
            NativeValue::Byte(2)
        )
        .is_err());
}

#[test]
fn a_fill_layer_path_and_its_pixels_round_trip_in_version_27() {
    let mut doc = Document::with_tile_size(32, 32, 16).unwrap();
    let fill = doc
        .add_fill_layer(
            "塗り",
            &[(Channel::Roughness, Rgba8::new(90, 90, 90, 255))],
            None,
        )
        .unwrap();
    doc.set_canvas_paths(
        fill,
        vec![entry(
            canvas(1, 6.5, Rgba8::new(10, 10, 200, 255)),
            "",
            true,
        )],
    )
    .unwrap();
    let native = NativeDocument::from_core(&doc).unwrap();
    assert_eq!(
        native.version(),
        PATHS_VERSION,
        "塗りつぶしの層のパスは 1 本でも一覧の形"
    );
    assert_eq!(native.field("layers[0].kind"), Some(&NativeValue::Int(1)));
    let back = native.to_core().unwrap();
    assert_eq!(paths_of(&back), paths_of(&doc));
    assert_eq!(pixels_of(&back), pixels_of(&doc));
    for (x, y) in [(10, 6), (10, 20)] {
        assert_eq!(
            back.composite_pixel(Channel::Color, x, y).unwrap(),
            doc.composite_pixel(Channel::Color, x, y).unwrap()
        );
    }
    assert!(back.composite_pixel(Channel::Color, 10, 6).unwrap().a > 0);
    assert_eq!(
        NativeDocument::from_core(&back).unwrap().to_bytes(),
        native.to_bytes()
    );
}

#[test]
fn a_fill_layer_3d_path_is_dropped_with_its_surface_before_it_is_put_on_the_shelf() {
    let g = SurfaceGeometry::new(
        vec![SurfaceTriangle::new(
            Vec3::ZERO,
            Vec3::X,
            Vec3::new(1.0, 1.0, 0.0),
            Vec2::ZERO,
            Vec2::X,
            Vec2::ONE,
        )],
        1,
        DEFAULT_WELD_TOLERANCE,
    )
    .unwrap();
    let path = SurfacePath {
        style: Default::default(),
        id: 5,
        channel: Channel::Roughness,
        brush: PathBrush(BrushSettings {
            radius: 0.2,
            ..BrushSettings::default()
        }),
        points: vec![PathPoint::new(0, 0.3, 0.2, 1.0).unwrap()],
        model_fingerprint: fingerprint(&g),
        material: None,
    };
    let entries = vec![LayerPathEntry::new(LayerPath::Surface(path))];
    let drawn = render_list(
        &entries,
        Some(&g),
        &Options {
            width: 32,
            height: 32,
            tile_size: 16,
            ..Options::default()
        },
    )
    .unwrap();
    let mut doc = Document::with_tile_size(32, 32, 16).unwrap();
    let fill = doc
        .add_fill_layer("塗り", &[(Channel::Color, Rgba8::new(9, 9, 9, 255))], None)
        .unwrap();
    doc.set_paths(fill, entries, drawn.channels).unwrap();
    let material = doc.capture_smart_material(&[fill], "素材").unwrap();
    assert!(
        material.notes().iter().any(|n| n.contains("外れ")),
        "{:?}",
        material.notes()
    );
    // 画素を持てない層の面が残っていれば、ここは「画素はラスターの層だけが持てます」で落ちる
    let file = SmartFile::from_core(&material, &writer()).unwrap();
    assert_eq!(
        file.fragment().to_core().unwrap().layers()[0].paths().len(),
        0
    );
    // 塗りつぶしの層の 2D のパスは一覧の形になるので、棚には入れず、理由を言う
    let mut doc = Document::with_tile_size(32, 32, 16).unwrap();
    let fill = doc
        .add_fill_layer("塗り", &[(Channel::Color, Rgba8::new(9, 9, 9, 255))], None)
        .unwrap();
    doc.set_canvas_paths(
        fill,
        vec![entry(
            canvas(1, 6.5, Rgba8::new(10, 10, 200, 255)),
            "",
            true,
        )],
    )
    .unwrap();
    let material = doc.capture_smart_material(&[fill], "素材").unwrap();
    let err = SmartFile::from_core(&material, &writer()).unwrap_err();
    assert!(err.to_string().contains(REFUSAL_PATH_LISTS), "{err}");
}
