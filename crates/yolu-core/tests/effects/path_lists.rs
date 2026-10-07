//! 層のパスの一覧: 1 本の一覧は今の 1 本のパスと同じ画素、後のパスが前のパスの上に重なる・消しゴムのパスは前のパスを消す・
//! 隠したパスは描かない。一覧の入れ替えは 1 回の Undo。上限・側・チャンネル・指紋・ID・名前を断る。複製は ID を付け直す。
use crate::attach_support;
use attach_support::*;
use yolu_core::geometry::{SurfaceGeometry, SurfaceTriangle, DEFAULT_WELD_TOLERANCE};
use yolu_core::glam::{Vec2, Vec3};
use yolu_core::paths::{
    fingerprint, list_channels, render_canvas, render_list, render_surface, validate_list,
    CanvasPath, CanvasPoint, ChannelPaint, Error, LayerPathEntry, Options, PathBrush, PathPoint,
    SurfacePath, MAX_LAYER_PATHS,
};
use yolu_core::{BrushSettings, Channel, Document, LayerId, LayerPath, Rgba8};

fn options() -> Options<'static> {
    Options {
        width: W,
        height: H,
        tile_size: 8,
        ..Options::default()
    }
}

fn brush(radius: f64, color: Rgba8) -> PathBrush {
    PathBrush(BrushSettings {
        radius,
        hardness: 1.0,
        spacing: 0.17,
        color,
        ..BrushSettings::default()
    })
}

fn canvas(id: u128, points: &[(f64, f64)], color: Rgba8) -> CanvasPath {
    CanvasPath {
        style: Default::default(),
        id,
        channel: Channel::Color,
        brush: brush(3.0, color),
        points: points
            .iter()
            .map(|&(x, y)| CanvasPoint::new(x, y, 1.0).unwrap())
            .collect(),
        material: None,
    }
}

fn entry(p: CanvasPath) -> LayerPathEntry {
    LayerPathEntry::new(LayerPath::Canvas(p))
}

fn bytes(r: &[(Channel, yolu_core::Surface)], c: Channel) -> Vec<u8> {
    r.iter()
        .find(|(rc, _)| *rc == c)
        .unwrap()
        .1
        .to_canvas_bytes()
}

fn layer_bytes(doc: &Document, layer: LayerId, c: Channel) -> Vec<u8> {
    doc.layer(layer)
        .unwrap()
        .surface(c)
        .unwrap()
        .to_canvas_bytes()
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

fn surface(id: u128, g: &SurfaceGeometry, material: Option<Vec<ChannelPaint>>) -> SurfacePath {
    SurfacePath {
        style: Default::default(),
        id,
        channel: Channel::Color,
        brush: brush(0.08, Rgba8::new(10, 200, 30, 255)),
        points: vec![
            PathPoint::new(0, 0.2, 0.3, 1.0).unwrap(),
            PathPoint::new(1, 0.4, 0.2, 0.6).unwrap(),
            PathPoint::new(1, 0.1, 0.7, 1.0).unwrap(),
        ],
        model_fingerprint: fingerprint(g),
        material,
    }
}

#[test]
fn a_list_of_one_path_draws_the_same_bytes_as_the_single_path() {
    let p = canvas(
        1,
        &[(4.5, 7.25), (29.5, 21.5), (37.25, 5.75)],
        Rgba8::new(201, 37, 89, 219),
    );
    let single = render_canvas(&p, &options()).unwrap();
    let list = render_list(&[entry(p.clone())], None, &options()).unwrap();
    assert_eq!(list.samples, single.samples);
    assert_eq!(
        bytes(&list.channels, Channel::Color),
        bytes(&single.channels, Channel::Color)
    );
    let g = plane();
    let s = surface(2, &g, None);
    let single = render_surface(&s, &g, &options()).unwrap();
    let list = render_list(
        &[LayerPathEntry::new(LayerPath::Surface(s.clone()))],
        Some(&g),
        &options(),
    )
    .unwrap();
    assert_eq!((list.dabs, list.gaps), (single.dabs, single.gaps));
    assert_eq!(
        bytes(&list.channels, Channel::Color),
        bytes(&single.channels, Channel::Color)
    );
    // 3D のパスは形が無ければ描かない
    assert!(matches!(
        render_list(
            &[LayerPathEntry::new(LayerPath::Surface(s))],
            None,
            &options()
        ),
        Err(Error::Invalid(_))
    ));
}

#[test]
fn later_paths_lie_on_earlier_ones_an_eraser_path_erases_them_and_hidden_paths_do_not_draw() {
    let red = Rgba8::new(255, 0, 0, 255);
    let blue = Rgba8::new(0, 0, 255, 255);
    let across = canvas(1, &[(2.5, 14.5), (37.5, 14.5)], red);
    let down = canvas(2, &[(20.5, 2.5), (20.5, 25.5)], blue);
    let r = render_list(
        &[entry(across.clone()), entry(down.clone())],
        None,
        &options(),
    )
    .unwrap();
    let c = bytes(&r.channels, Channel::Color);
    let px = |b: &[u8], x: u32, y: u32| {
        let i = ((y * W + x) * 4) as usize;
        [b[i], b[i + 1], b[i + 2], b[i + 3]]
    };
    assert_eq!(px(&c, 20, 14), [0, 0, 255, 255], "後のパスが上");
    assert_eq!(px(&c, 6, 14)[0], 255, "前のパスは重ならない所に残る");
    // 順を入れ替えると上下も入れ替わる
    let r = render_list(
        &[entry(down.clone()), entry(across.clone())],
        None,
        &options(),
    )
    .unwrap();
    assert_eq!(
        px(&bytes(&r.channels, Channel::Color), 20, 14),
        [255, 0, 0, 255]
    );
    // 消しゴムのパスは前のパスの画素を消す
    let mut eraser = canvas(3, &[(20.5, 2.5), (20.5, 25.5)], red);
    eraser.brush.0.erase = true;
    let r = render_list(&[entry(across.clone()), entry(eraser)], None, &options()).unwrap();
    let c = bytes(&r.channels, Channel::Color);
    assert_eq!(px(&c, 20, 14)[3], 0, "消した");
    assert_eq!(px(&c, 6, 14)[3], 255, "消しゴムの外は残る");
    // 隠したパスは描かないが、チャンネルは返す（空の面）
    let mut hidden = entry(across.clone());
    hidden.visible = false;
    let r = render_list(&[hidden.clone()], None, &options()).unwrap();
    assert_eq!(r.channels.len(), 1);
    assert_eq!(r.channels[0].1.tile_count(), 0);
    let r = render_list(&[hidden, entry(down.clone())], None, &options()).unwrap();
    let alone = render_canvas(&down, &options()).unwrap();
    assert_eq!(
        bytes(&r.channels, Channel::Color),
        bytes(&alone.channels, Channel::Color)
    );
}

#[test]
fn the_list_is_checked_before_drawing() {
    let a = canvas(1, &[(2.5, 2.5)], Rgba8::new(1, 2, 3, 255));
    let ok = |e: &[LayerPathEntry]| validate_list(e).is_ok();
    assert!(ok(&[]));
    assert!(ok(&[entry(a.clone())]));
    assert!(!ok(&[entry(a.clone()), entry(a.clone())]), "ID が重なる");
    let mut b = canvas(2, &[(5.5, 5.5)], Rgba8::new(1, 2, 3, 255));
    b.channel = Channel::Roughness;
    assert!(!ok(&[entry(a.clone()), entry(b)]), "基準のチャンネルが違う");
    let g = plane();
    assert!(
        !ok(&[
            entry(a.clone()),
            LayerPathEntry::new(LayerPath::Surface(surface(3, &g, None)))
        ]),
        "キャンバスとモデルの上が混ざる"
    );
    let mut other = surface(4, &g, None);
    other.model_fingerprint = "00".repeat(16);
    assert!(
        !ok(&[
            LayerPathEntry::new(LayerPath::Surface(surface(3, &g, None))),
            LayerPathEntry::new(LayerPath::Surface(other))
        ]),
        "別のモデル"
    );
    let mut named = entry(a.clone());
    named.name = "名".repeat(128);
    assert!(ok(std::slice::from_ref(&named)));
    named.name = "名".repeat(129);
    assert!(!ok(std::slice::from_ref(&named)), "名前が長すぎる");
    named.name = "a\nb".into();
    assert!(!ok(std::slice::from_ref(&named)), "制御文字");
    let many: Vec<LayerPathEntry> = (0..=MAX_LAYER_PATHS as u128)
        .map(|i| entry(canvas(i + 10, &[], Rgba8::new(1, 2, 3, 255))))
        .collect();
    assert!(ok(&many[..MAX_LAYER_PATHS]));
    assert!(!ok(&many), "257 本は断る");
    assert!(render_list(&many, None, &options()).is_err());
}

#[test]
fn replacing_the_list_is_one_undo_step_and_clears_channels_no_path_draws_anymore() {
    let (mut doc, _) = world();
    let layer = doc.add_layer("パス").unwrap();
    let red = Rgba8::new(255, 0, 0, 255);
    let mut a = canvas(1, &[(2.5, 14.5), (37.5, 14.5)], red);
    a.material = Some(vec![
        ChannelPaint {
            channel: Channel::Color,
            color: red,
        },
        ChannelPaint {
            channel: Channel::Roughness,
            color: Rgba8::new(90, 90, 90, 255),
        },
    ]);
    let b = canvas(2, &[(20.5, 2.5), (20.5, 25.5)], Rgba8::new(0, 0, 255, 255));
    let steps = doc.undo_count();
    doc.set_canvas_paths(layer, vec![entry(a.clone()), entry(b.clone())])
        .unwrap();
    assert_eq!(doc.undo_count(), steps + 1);
    let l = doc.layer(layer).unwrap();
    assert_eq!(l.paths().len(), 2);
    assert_eq!(
        list_channels(l.paths()),
        vec![Channel::Color, Channel::Roughness]
    );
    let both = layer_bytes(&doc, layer, Channel::Color);
    let rough = layer_bytes(&doc, layer, Channel::Roughness);
    assert!(rough.iter().any(|v| *v != 0));
    // 組を持つパスを消すと、もう誰も描かない Roughness は空になる
    doc.set_canvas_paths(layer, vec![entry(b.clone())]).unwrap();
    let l = doc.layer(layer).unwrap();
    assert_eq!(l.surface(Channel::Roughness).unwrap().tile_count(), 0);
    assert_eq!(
        layer_bytes(&doc, layer, Channel::Color),
        bytes(
            &render_canvas(&b, &options()).unwrap().channels,
            Channel::Color
        )
    );
    doc.undo().unwrap();
    assert_eq!(doc.layer(layer).unwrap().paths().len(), 2);
    assert_eq!(layer_bytes(&doc, layer, Channel::Color), both);
    assert_eq!(layer_bytes(&doc, layer, Channel::Roughness), rough);
    // 1 本のパスを付け直すと、同じ ID の名前と表示を引き継ぐ
    let mut named = vec![entry(a.clone())];
    named[0].name = "縁".into();
    doc.set_canvas_paths(layer, named).unwrap();
    let mut moved = a.clone();
    moved.points[0].y = 10.5;
    doc.set_canvas_path(layer, moved).unwrap();
    assert_eq!(doc.layer(layer).unwrap().paths()[0].name, "縁");
    // 空の一覧はパスを外し、そのチャンネルを空にする（手で描ける層に戻る）
    doc.set_canvas_paths(layer, Vec::new()).unwrap();
    let l = doc.layer(layer).unwrap();
    assert!(!l.has_paths());
    assert_eq!(l.surface(Channel::Color).unwrap().tile_count(), 0);
    doc.undo().unwrap();
    assert!(doc.layer(layer).unwrap().has_paths());
    // 一覧を変えても、層の側と基準のチャンネルは変えない
    let mut rough_base = canvas(5, &[(2.5, 2.5)], red);
    rough_base.channel = Channel::Roughness;
    assert!(doc
        .set_canvas_paths(layer, vec![entry(rough_base)])
        .is_err());
}

#[test]
fn renaming_a_path_changes_only_the_name_in_one_undo_step_without_redrawing() {
    let (mut doc, _) = world();
    let layer = doc.add_layer("パス").unwrap();
    let a = canvas(1, &[(2.5, 14.5), (37.5, 14.5)], Rgba8::new(255, 0, 0, 255));
    let b = canvas(2, &[(20.5, 2.5), (20.5, 25.5)], Rgba8::new(0, 0, 255, 255));
    doc.set_canvas_paths(layer, vec![entry(a.clone()), entry(b.clone())])
        .unwrap();
    let pixels = layer_bytes(&doc, layer, Channel::Color);
    let (steps, revision) = (doc.undo_count(), doc.revision());
    doc.rename_path(layer, 2, "縦").unwrap();
    assert_eq!(doc.undo_count(), steps + 1, "1 回の Undo");
    assert_ne!(doc.revision(), revision);
    let l = doc.layer(layer).unwrap();
    assert_eq!(l.paths()[0].name, "");
    assert_eq!(l.paths()[1].name, "縦");
    assert_eq!(
        l.paths()[1].path,
        LayerPath::Canvas(b.clone()),
        "パスは同じ"
    );
    assert_eq!(
        layer_bytes(&doc, layer, Channel::Color),
        pixels,
        "画素は同じ"
    );
    // 同じ名前は何もしない。ないパス・決まりに合わない名前は断って何も変えない
    doc.rename_path(layer, 2, "縦").unwrap();
    assert_eq!(doc.undo_count(), steps + 1);
    assert!(doc.rename_path(layer, 99, "x").is_err());
    assert!(doc.rename_path(layer, 2, "a\nb").is_err(), "制御文字");
    assert!(doc.rename_path(layer, 2, &"長".repeat(129)).is_err());
    assert_eq!(doc.undo_count(), steps + 1);
    assert_eq!(doc.layer(layer).unwrap().paths()[1].name, "縦");
    // ロックの「すべて」では断る（画素を変えない操作の決めは、パスを画素にするときと同じ）
    doc.set_layer_locks(layer, yolu_core::LayerLocks::ALL)
        .unwrap();
    let locked = doc.undo_count();
    assert!(doc.rename_path(layer, 2, "横").is_err());
    assert_eq!(doc.undo_count(), locked);
    doc.undo().unwrap();
    doc.undo().unwrap();
    assert_eq!(doc.layer(layer).unwrap().paths()[1].name, "");
    assert_eq!(layer_bytes(&doc, layer, Channel::Color), pixels);
    doc.redo().unwrap();
    doc.redo().unwrap();
    assert_eq!(doc.layer(layer).unwrap().paths()[1].name, "縦");
}

#[test]
fn duplicating_a_path_layer_gives_every_path_a_new_id() {
    let (mut doc, _) = world();
    let layer = doc.add_layer("パス").unwrap();
    let a = canvas(1, &[(2.5, 14.5), (37.5, 14.5)], Rgba8::new(255, 0, 0, 255));
    let b = canvas(2, &[(20.5, 2.5), (20.5, 25.5)], Rgba8::new(0, 0, 255, 255));
    doc.set_canvas_paths(layer, vec![entry(a), entry(b)])
        .unwrap();
    let copy = doc.duplicate_layer(layer, None).unwrap();
    let ids = |id: LayerId| -> Vec<u128> {
        doc.layer(id)
            .unwrap()
            .paths()
            .iter()
            .map(LayerPathEntry::id)
            .collect()
    };
    let (old, new) = (ids(layer), ids(copy));
    assert_eq!(new.len(), 2);
    assert_ne!(new[0], new[1]);
    assert!(new.iter().all(|i| !old.contains(i)));
    assert_eq!(
        layer_bytes(&doc, layer, Channel::Color),
        layer_bytes(&doc, copy, Channel::Color)
    );
}

#[test]
fn resizing_the_canvas_redraws_every_path_of_the_list() {
    let (mut doc, _) = world();
    let layer = doc.add_layer("パス").unwrap();
    let a = canvas(1, &[(2.5, 14.5), (37.5, 14.5)], Rgba8::new(255, 0, 0, 255));
    let mut b = canvas(2, &[(20.5, 2.5), (20.5, 25.5)], Rgba8::new(0, 0, 255, 255));
    let mut hidden = entry(b.clone());
    hidden.visible = false;
    b.id = 3;
    doc.set_canvas_paths(layer, vec![entry(a), hidden, entry(b)])
        .unwrap();
    doc.resize_canvas(W * 2, H * 2, (0, 0)).unwrap();
    let l = doc.layer(layer).unwrap();
    assert_eq!(l.paths().len(), 3);
    assert!(!l.paths()[1].visible, "表示はそのまま");
    let expected = render_list(
        l.paths(),
        None,
        &Options {
            width: W * 2,
            height: H * 2,
            tile_size: 8,
            ..Options::default()
        },
    )
    .unwrap();
    assert_eq!(
        layer_bytes(&doc, layer, Channel::Color),
        bytes(&expected.channels, Channel::Color)
    );
}
