//! パスのブラシの縁のアンチエイリアス（正本の版 34、パスの `brush` の塊の `anti_alias`）。なし のパスだけの文書は前の版のまま欄を書かず、
//! なし でないパスのある文書だけが版 34 になる。往復・版の選び方・古い読み手の範囲・範囲の外の値の拒否を試す。
use yolu_core::geometry::{SurfaceGeometry, SurfaceTriangle, DEFAULT_WELD_TOLERANCE};
use yolu_core::glam::{Vec2, Vec3};
use yolu_core::paths::{
    fingerprint, render_list, CanvasPath, CanvasPoint, LayerPathEntry, Options, PathBrush,
    PathPoint, SurfacePath,
};
use yolu_core::{AntiAlias, BrushSettings, Channel, Document, LayerPath, Rgba8};
use yolu_io::{
    NativeDocument, NativeValue, ANTI_ALIAS_VERSION, BAKE_PRIORITY_VERSION, PATHS_VERSION,
    UNITY_NATIVE_VERSION,
};

fn canvas(id: u128, level: AntiAlias) -> CanvasPath {
    CanvasPath {
        style: Default::default(),
        id,
        channel: Channel::Color,
        brush: PathBrush(BrushSettings {
            radius: 2.25,
            hardness: 1.0,
            color: Rgba8::new(20, 30, 200, 255),
            anti_alias: level,
            ..BrushSettings::default()
        }),
        points: vec![
            CanvasPoint::new(3.5, 4.0, 1.0).unwrap(),
            CanvasPoint::new(27.25, 21.5, 1.0).unwrap(),
        ],
        material: None,
    }
}

fn doc_with(entries: Vec<(CanvasPath, &str)>) -> Document {
    let mut doc = Document::with_tile_size(32, 32, 16).unwrap();
    let layer = doc.add_layer("パス").unwrap();
    let entries = entries
        .into_iter()
        .map(|(p, name)| LayerPathEntry {
            name: name.into(),
            visible: true,
            path: LayerPath::Canvas(p),
        })
        .collect();
    doc.set_canvas_paths(layer, entries).unwrap();
    doc
}

fn levels_of(doc: &Document) -> Vec<AntiAlias> {
    doc.layers()
        .iter()
        .flat_map(|l| l.paths().iter())
        .map(|e| match &e.path {
            LayerPath::Canvas(c) => c.brush.0.anti_alias,
            LayerPath::Surface(s) => s.brush.0.anti_alias,
        })
        .collect()
}

fn pixels_of(doc: &Document) -> Vec<u8> {
    let l = &doc.layers()[0];
    l.surface(Channel::Color).unwrap().to_canvas_bytes()
}

#[test]
fn only_a_document_with_an_anti_aliased_path_is_version_34() {
    assert_eq!(ANTI_ALIAS_VERSION, 34);
    // なし: 版も並びも前のまま（1 本の欄、Unity 版が読める 21）
    let plain = doc_with(vec![(canvas(1, AntiAlias::None), "")]);
    let native = NativeDocument::from_core(&plain).unwrap();
    assert_eq!(native.version(), UNITY_NATIVE_VERSION);
    assert!(native
        .fields()
        .iter()
        .all(|f| !f.path.ends_with("anti_alias")));
    assert_eq!(levels_of(&native.to_core().unwrap()), [AntiAlias::None]);
    // 段のある 1 本は版 34 で、欄を持って往復する（画素も同じ）
    for level in [AntiAlias::Weak, AntiAlias::Medium, AntiAlias::Strong] {
        let doc = doc_with(vec![(canvas(1, level), "")]);
        assert_ne!(
            pixels_of(&doc),
            pixels_of(&plain),
            "{level:?} でも なし と同じ画素"
        );
        let native = NativeDocument::from_core(&doc).unwrap();
        assert_eq!(native.version(), ANTI_ALIAS_VERSION);
        assert_eq!(
            native.field("layers[0].canvas_path.brush.anti_alias"),
            Some(&NativeValue::Byte(level.index()))
        );
        let back = native.to_core().unwrap();
        assert_eq!(levels_of(&back), [level]);
        assert_eq!(pixels_of(&back), pixels_of(&doc));
        assert_eq!(
            NativeDocument::from_core(&back).unwrap().to_bytes(),
            native.to_bytes()
        );
    }
    // 一覧の形（名前を付けたパス）も、段のあるパスがあれば 34、無ければ 27
    let listed = doc_with(vec![
        (canvas(1, AntiAlias::None), "下"),
        (canvas(2, AntiAlias::Medium), "上"),
    ]);
    let native = NativeDocument::from_core(&listed).unwrap();
    assert_eq!(native.version(), ANTI_ALIAS_VERSION);
    assert_eq!(
        native.field("layers[0].paths.items[0].path.brush.anti_alias"),
        Some(&NativeValue::Byte(0))
    );
    assert_eq!(
        native.field("layers[0].paths.items[1].path.brush.anti_alias"),
        Some(&NativeValue::Byte(2))
    );
    assert_eq!(
        levels_of(&native.to_core().unwrap()),
        [AntiAlias::None, AntiAlias::Medium]
    );
    let listed_plain = doc_with(vec![
        (canvas(1, AntiAlias::None), "下"),
        (canvas(2, AntiAlias::None), "上"),
    ]);
    assert_eq!(
        NativeDocument::from_core(&listed_plain).unwrap().version(),
        PATHS_VERSION
    );
}

#[test]
fn version_34_is_outside_what_older_readers_accept() {
    let native =
        NativeDocument::from_core(&doc_with(vec![(canvas(1, AntiAlias::Strong), "")])).unwrap();
    // 0.5.x の読み手の上限は版 33。版の数だけを 33 にした版 34 の正本は、欄が余って読めない
    let mut bytes = native.to_bytes();
    assert_eq!(&bytes[8..12], &ANTI_ALIAS_VERSION.to_le_bytes());
    bytes[8..12].copy_from_slice(&BAKE_PRIORITY_VERSION.to_le_bytes());
    assert!(NativeDocument::read(&bytes).is_err());
    // 版 21 の正本の版の数だけを 34 にしても、欄が足りず読めない
    let mut bytes = NativeDocument::from_core(&doc_with(vec![(canvas(1, AntiAlias::None), "")]))
        .unwrap()
        .to_bytes();
    bytes[8..12].copy_from_slice(&ANTI_ALIAS_VERSION.to_le_bytes());
    assert!(NativeDocument::read(&bytes).is_err());
}

#[test]
fn out_of_range_levels_are_refused_and_every_level_is_read() {
    let native =
        NativeDocument::from_core(&doc_with(vec![(canvas(1, AntiAlias::Weak), "")])).unwrap();
    for bad in [4u8, 255] {
        assert!(
            native
                .with_value(
                    "layers[0].canvas_path.brush.anti_alias",
                    NativeValue::Byte(bad)
                )
                .is_err(),
            "{bad} を通した"
        );
    }
    for level in AntiAlias::ALL {
        let changed = native
            .with_value(
                "layers[0].canvas_path.brush.anti_alias",
                NativeValue::Byte(level.index()),
            )
            .unwrap();
        assert_eq!(levels_of(&changed.to_core().unwrap()), [level]);
    }
}

#[test]
fn a_3d_path_keeps_its_level_and_pixels_through_version_34() {
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
    let options = Options {
        width: 32,
        height: 32,
        tile_size: 16,
        ..Options::default()
    };
    let doc_of = |level: AntiAlias| {
        let path = SurfacePath {
            style: Default::default(),
            id: 9,
            channel: Channel::Color,
            brush: PathBrush(BrushSettings {
                radius: 0.08,
                hardness: 1.0,
                anti_alias: level,
                ..BrushSettings::default()
            }),
            points: vec![
                PathPoint::new(0, 0.2, 0.1, 1.0).unwrap(),
                PathPoint::new(1, 0.4, 0.3, 1.0).unwrap(),
            ],
            model_fingerprint: fingerprint(&g),
            material: None,
        };
        let entries = vec![LayerPathEntry::new(LayerPath::Surface(path))];
        let drawn = render_list(&entries, Some(&g), &options).unwrap();
        let mut doc = Document::with_tile_size(32, 32, 16).unwrap();
        let layer = doc.add_layer("面").unwrap();
        doc.set_paths(layer, entries, drawn.channels).unwrap();
        doc
    };
    let plain = doc_of(AntiAlias::None);
    assert_eq!(
        NativeDocument::from_core(&plain).unwrap().version(),
        UNITY_NATIVE_VERSION
    );
    for level in [AntiAlias::Weak, AntiAlias::Medium, AntiAlias::Strong] {
        let doc = doc_of(level);
        // 3D のパスの丸い筆先にも帯が付く
        assert_ne!(pixels_of(&doc), pixels_of(&plain), "{level:?}");
        let native = NativeDocument::from_core(&doc).unwrap();
        assert_eq!(native.version(), ANTI_ALIAS_VERSION);
        assert_eq!(
            native.field("layers[0].surface_path.brush.anti_alias"),
            Some(&NativeValue::Byte(level.index()))
        );
        let back = native.to_core().unwrap();
        assert_eq!(levels_of(&back), [level]);
        assert_eq!(pixels_of(&back), pixels_of(&doc));
        assert_eq!(
            NativeDocument::from_core(&back).unwrap().to_bytes(),
            native.to_bytes()
        );
    }
}
