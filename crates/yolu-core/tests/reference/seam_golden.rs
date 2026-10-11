//! 効果（フィルター・Generator・Anchor・塗りつぶしの画像・パス）とレイヤーのロック・レイヤーの操作のつなぎ目を、実 C# の PaintDocument の結果と
//! 全バイトで照らす。台本・人工の文書・書き出しの並びは tools/csharp-golden/SeamGolden.cs と対（事例ごとに SHA-256 を 1 行、
//! golden/seam.txt）。食い違ったときは `SEAM_DUMP_DIR=<dir>` で食い違った事例の生のバイト列を書き出し、
//! `tools/csharp-golden/run-seam.sh dump <名前> <出力>` の結果と cmp で比べる。
//! メッシュマップは渡さない（Generator の Anchor は使える。マップを要る段は入力のまま通す＝効いていない効果）。
use crate::attach_support;
use crate::golden_update;
use crate::seam_support;
use attach_support::{paint, H, W};
use seam_support::{path_points, rendered};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use yolu_core::fill_image::{Projection, ProjectionMode};
use yolu_core::generator::{self, anchor::ReadMode, Settings};
use yolu_core::material::{ChannelPaint, GradientSettings};
use yolu_core::paths::{PathBrush, PathPoint, SurfacePath};
use yolu_core::{
    Affine2D, AnchorId, AnchorPlacement, BrushSettings, CanvasResampling, Channel, CoreError,
    Document, EffectInputs, EffectSettings, FilterId, FilterSpec, FilterTarget, ImageColorSpace,
    ImageId, ImageInput, LayerId, LayerLocks, LayerMergeReport, LayerPath, Resampling, Rgba8,
    SelectionMask,
};

// ───────── 書き出し（.NET の BinaryWriter と同じ並び） ─────────

fn i32b(o: &mut Vec<u8>, n: i32) {
    o.extend_from_slice(&n.to_le_bytes());
}
fn u64b(o: &mut Vec<u8>, n: u64) {
    o.extend_from_slice(&n.to_le_bytes());
}
fn f64b(o: &mut Vec<u8>, n: f64) {
    o.extend_from_slice(&n.to_le_bytes());
}
fn boolb(o: &mut Vec<u8>, b: bool) {
    o.push(b as u8);
}
/// BinaryWriter.Write(string): 長さ（127 まで）+ UTF-8。
fn stringb(o: &mut Vec<u8>, s: &str) {
    assert!(s.len() < 128);
    o.push(s.len() as u8);
    o.extend_from_slice(s.as_bytes());
}

/// 効果と Anchor の ID を、最初に見た順の番号にする（Rust と C# の ID は別物なので、同じものを指すかだけを比べる）。
#[derive(Default)]
struct Ids {
    filters: HashMap<u128, i32>,
    anchors: HashMap<u128, i32>,
}
impl Ids {
    fn filter(&mut self, id: u128) -> i32 {
        if id == 0 {
            return -1;
        }
        let n = self.filters.len() as i32;
        *self.filters.entry(id).or_insert(n)
    }
    fn anchor(&mut self, id: u128) -> i32 {
        if id == 0 {
            return -1;
        }
        let n = self.anchors.len() as i32;
        *self.anchors.entry(id).or_insert(n)
    }
}

fn effects(o: &mut Vec<u8>, list: &[yolu_core::FilterEffect], ids: &mut Ids) {
    i32b(o, list.len() as i32);
    for e in list {
        let s = e.settings();
        i32b(o, s.type_index());
        let radius = match s {
            EffectSettings::Filter(yolu_core::filter::Settings::GaussianBlur { radius })
            | EffectSettings::Filter(yolu_core::filter::Settings::Sharpen { radius, .. }) => {
                *radius as i32
            }
            _ => 0,
        };
        i32b(o, radius);
        boolb(o, e.enabled());
        f64b(o, e.strength());
        i32b(o, e.channels().iter().fold(0, |m, c| m | 1 << c.index()));
        i32b(o, ids.filter(e.id().0));
        if let Some(g) = s.generator_settings() {
            i32b(o, g.kind as i32);
            if g.kind == generator::Kind::Anchor {
                i32b(o, ids.anchor(g.anchor.id));
                i32b(o, g.anchor.channel.index() as i32);
                i32b(o, g.anchor.read as i32);
            }
        }
    }
}
fn anchor_of(o: &mut Vec<u8>, a: Option<&yolu_core::Anchor>, placement: i32, ids: &mut Ids) {
    boolb(o, a.is_some());
    if let Some(a) = a {
        i32b(o, ids.anchor(a.id().0));
        stringb(o, a.name());
        i32b(o, placement);
    }
}
fn snapshot(o: &mut Vec<u8>, d: &Document, ids: &mut Ids) {
    i32b(o, d.width() as i32);
    i32b(o, d.height() as i32);
    i32b(o, d.layers().len() as i32);
    for l in d.layers() {
        i32b(o, l.kind() as i32);
        i32b(
            o,
            l.parent().map_or(-1, |p| d.layer_index(p).unwrap() as i32),
        );
        boolb(o, l.visible());
        f64b(o, l.opacity());
        i32b(o, l.blend_mode() as i32);
        boolb(o, l.clipping());
        i32b(o, l.locks().bits() as i32);
        for c in Channel::ALL {
            boolb(o, l.is_channel_enabled(c));
        }
        boolb(o, l.path().is_some());
        if let Some(p) = l.path() {
            match p {
                LayerPath::Canvas(p) => {
                    i32b(o, p.channel.index() as i32);
                    i32b(o, p.points.len() as i32);
                    boolb(o, true);
                    f64b(o, p.brush.0.radius);
                    for q in &p.points {
                        f64b(o, q.x);
                        f64b(o, q.y);
                        f64b(o, q.pressure);
                    }
                }
                LayerPath::Surface(p) => {
                    i32b(o, p.channel.index() as i32);
                    i32b(o, p.points.len() as i32);
                    boolb(o, false);
                }
            }
        }
        effects(o, l.filters(), ids);
        anchor_of(o, l.anchor(), 0, ids);
        boolb(o, l.mask().is_some());
        if let Some(m) = l.mask() {
            boolb(o, m.enabled());
            boolb(o, m.inverted());
            f64b(o, m.density());
            effects(o, m.filters(), ids);
            anchor_of(o, m.anchor(), 1, ids);
        }
    }
    for c in Channel::ALL {
        o.extend(d.composite_channel(c, d.bounds()).unwrap());
    }
}
fn report(o: &mut Vec<u8>, r: &LayerMergeReport) {
    i32b(o, r.method as i32);
    i32b(o, r.notes as i32);
    u64b(o, r.compared_pixels);
    u64b(o, r.changed_pixels);
    i32b(o, r.max_difference as i32);
    i32b(o, r.max_visible_difference as i32);
    for c in Channel::ALL {
        u64b(o, r.changed_by_channel.get(&c).copied().unwrap_or(0));
    }
}
/// 結果の型: 0 成功、1 ロックで断られた（レイヤー・持ち主・ロック）、2 そのほか（理由の名前）。成功なら true。
fn outcome<T>(o: &mut Vec<u8>, d: &Document, r: Result<T, CoreError>) -> bool {
    match r {
        Ok(_) => {
            o.push(0);
            true
        }
        Err(CoreError::LayerLocked {
            layer,
            holder,
            lock,
        }) => {
            o.push(1);
            // レイヤーが文書に無いこともある（結合や新しいレイヤーの追加が断られて、作りかけのレイヤーが消えた）。C# の FindIndex も -1
            i32b(o, d.layer_index(layer).map_or(-1, |i| i as i32));
            i32b(o, d.layer_index(holder).map_or(-1, |i| i as i32));
            i32b(o, lock.bits() as i32);
            false
        }
        Err(e) => {
            o.push(2);
            stringb(
                o,
                &match e {
                    CoreError::MergeRefused(r) => format!("op:{r:?}"),
                    CoreError::MergeAppearance(_) => "op:AppearanceChanges".to_owned(),
                    CoreError::Unsupported(_) | CoreError::InactiveEffect { .. } => {
                        "invalid-op".to_owned()
                    }
                    CoreError::InvalidArgument(_) => "argument".to_owned(),
                    other => format!("other:{other:?}"),
                },
            );
            false
        }
    }
}
fn after_op(o: &mut Vec<u8>, d: &mut Document, ids: &mut Ids, ok: bool) {
    u64b(o, d.history_bytes());
    snapshot(o, d, ids);
    if !ok {
        return;
    }
    d.undo().unwrap();
    u64b(o, d.history_bytes());
    snapshot(o, d, ids);
    d.redo().unwrap();
    u64b(o, d.history_bytes());
    snapshot(o, d, ids);
}

// ───────── 人工の文書 ─────────

const IMAGE: ImageId = ImageId(0x1000);

fn new_doc() -> Document {
    let mut doc = Document::with_tile_size(W, H, 8).unwrap();
    let (w, h) = (8u32, 6u32);
    let mut pixels = Vec::new();
    for y in 0..h {
        for x in 0..w {
            pixels.extend([
                (x * 31 + 20) as u8,
                (y * 40 + 10) as u8,
                if (x + y) % 2 == 0 { 220 } else { 60 },
                if (x + 2 * y) % 5 == 0 { 90 } else { 255 },
            ]);
        }
    }
    doc.set_effect_inputs(EffectInputs::new().with_image(
        IMAGE,
        ImageInput::new(w, h, pixels, ImageColorSpace::Srgb).unwrap(),
    ))
    .unwrap();
    doc
}
fn path_brush() -> PathBrush {
    PathBrush(BrushSettings {
        radius: 3.0,
        hardness: 0.7,
        spacing: 0.17,
        opacity: 0.83,
        flow: 0.42,
        color: Rgba8::new(201, 37, 89, 219),
        pressure_size: true,
        pressure_opacity: false,
        pressure_flow: false,
        erase: false,
        anti_alias: yolu_core::AntiAlias::None,
    })
}
fn canvas_path(shift: f64) -> yolu_core::paths::CanvasPath {
    let mut p = path_points(shift);
    p.brush = path_brush();
    p
}
fn anchor_generator() -> EffectSettings {
    let mut g = Settings::new(generator::Kind::Anchor);
    g.blend = generator::Blend::Replace;
    EffectSettings::generator(g)
}
fn gradient() -> Settings {
    let mut g = Settings::new(generator::Kind::ShapeGradient);
    g.ramp = Some(generator::Ramp::default());
    g.blend = generator::Blend::Replace;
    g
}

struct Rig {
    doc: Document,
    base: LayerId,
    mid: LayerId,
    top: LayerId,
    fill: LayerId,
    plain_fill: LayerId,
    path_layer: LayerId,
    blur: FilterId,
    mask_blur: FilterId,
    reader: FilterId,
    anchor: AnchorId,
}
/// 効果をひと通り持つ文書（SeamGolden.cs の MakeRig と同じ）。
fn rig() -> Rig {
    let mut doc = new_doc();
    let base = doc.add_layer("土台").unwrap();
    paint(&mut doc, base, Channel::Color, 1);
    paint(&mut doc, base, Channel::Height, 2);
    let mid = doc.add_layer("中").unwrap();
    paint(&mut doc, mid, Channel::Height, 3);
    let top = doc.add_layer("上").unwrap();
    paint(&mut doc, top, Channel::Color, 4);
    paint(&mut doc, top, Channel::Height, 5);
    let fill = doc
        .add_fill_layer(
            "塗り",
            &[
                (Channel::Color, Rgba8::new(60, 160, 220, 200)),
                (Channel::Height, Rgba8::new(128, 128, 128, 100)),
            ],
            None,
        )
        .unwrap();
    let blur = doc
        .add_filter(
            base,
            FilterTarget::Content,
            FilterSpec::new(EffectSettings::blur(3)).channels(&[Channel::Color]),
        )
        .unwrap();
    let anchor = doc
        .add_anchor(base, AnchorPlacement::Layer, Some("土台"), None)
        .unwrap();
    let reader = doc
        .add_filter(
            mid,
            FilterTarget::Content,
            FilterSpec::new(anchor_generator()).channels(&[Channel::Height]),
        )
        .unwrap();
    doc.set_generator_anchor(
        mid,
        reader,
        Some(anchor),
        Channel::Height,
        ReadMode::Value,
        false,
    )
    .unwrap();
    doc.add_layer_mask(top).unwrap();
    let mask_blur = doc
        .add_filter(
            top,
            FilterTarget::Mask,
            FilterSpec::new(EffectSettings::blur(2)),
        )
        .unwrap();
    doc.set_fill_image(fill, Channel::Color, Some(IMAGE))
        .unwrap();
    let plain_fill = doc
        .add_fill_layer(
            "素の塗り",
            &[(Channel::Color, Rgba8::new(5, 6, 7, 255))],
            None,
        )
        .unwrap();
    let path_layer = doc.add_layer("パス").unwrap();
    doc.set_canvas_path(path_layer, canvas_path(0.0)).unwrap();
    doc.clear_history().unwrap();
    Rig {
        doc,
        base,
        mid,
        top,
        fill,
        plain_fill,
        path_layer,
        blur,
        mask_blur,
        reader,
        anchor,
    }
}

// ───────── 効果の設定の入口 × ロック ─────────

type EntryRun = Box<dyn Fn(&mut Rig) -> Result<(), CoreError>>;

struct Entry {
    name: &'static str,
    target: fn(&Rig) -> LayerId,
    run: EntryRun,
}
fn e(
    name: &'static str,
    target: fn(&Rig) -> LayerId,
    run: impl Fn(&mut Rig) -> Result<(), CoreError> + 'static,
) -> Entry {
    Entry {
        name,
        target,
        run: Box::new(run),
    }
}
fn invert_height() -> FilterSpec {
    FilterSpec::new(EffectSettings::invert()).channels(&[Channel::Height])
}
fn entries() -> Vec<Entry> {
    vec![
        e(
            "add_filter",
            |r| r.base,
            |r| {
                r.doc
                    .add_filter(r.base, FilterTarget::Content, invert_height())
                    .map(|_| ())
            },
        ),
        e(
            "remove_filter",
            |r| r.base,
            |r| r.doc.remove_filter(r.base, r.blur),
        ),
        e(
            "set_filter_enabled",
            |r| r.base,
            |r| r.doc.set_filter_enabled(r.base, r.blur, false),
        ),
        e(
            "set_filter_strength",
            |r| r.base,
            |r| r.doc.set_filter_strength(r.base, r.blur, 0.5, false),
        ),
        e(
            "set_filter_settings",
            |r| r.base,
            |r| {
                r.doc
                    .set_filter_settings(r.base, r.blur, EffectSettings::blur(7), false)
            },
        ),
        e(
            "set_filter_channels",
            |r| r.base,
            |r| {
                r.doc
                    .set_filter_channels(r.base, r.blur, &[Channel::Color, Channel::Height])
            },
        ),
        e(
            "move_filter",
            |r| r.base,
            |r| {
                r.doc
                    .add_filter(r.base, FilterTarget::Content, invert_height())
                    .map(|_| ())?;
                r.doc.move_filter(r.base, r.blur, 1)
            },
        ),
        e(
            "mask_add_filter",
            |r| r.top,
            |r| {
                r.doc
                    .add_filter(
                        r.top,
                        FilterTarget::Mask,
                        FilterSpec::new(EffectSettings::invert()),
                    )
                    .map(|_| ())
            },
        ),
        e(
            "mask_set_filter_strength",
            |r| r.top,
            |r| r.doc.set_filter_strength(r.top, r.mask_blur, 0.4, false),
        ),
        e(
            "add_anchor",
            |r| r.mid,
            |r| {
                r.doc
                    .add_anchor(r.mid, AnchorPlacement::Layer, None, None)
                    .map(|_| ())
            },
        ),
        e(
            "add_anchor_mask",
            |r| r.top,
            |r| {
                r.doc
                    .add_anchor(r.top, AnchorPlacement::Mask, Some("上のマスク"), None)
                    .map(|_| ())
            },
        ),
        e(
            "remove_anchor",
            |r| r.base,
            |r| r.doc.remove_anchor(r.anchor),
        ),
        e(
            "rename_anchor",
            |r| r.base,
            |r| r.doc.rename_anchor(r.anchor, "別の名前"),
        ),
        e(
            "set_generator_anchor",
            |r| r.mid,
            |r| {
                r.doc.set_generator_anchor(
                    r.mid,
                    r.reader,
                    None,
                    Channel::Height,
                    ReadMode::Value,
                    false,
                )
            },
        ),
        e(
            "set_fill_image_off",
            |r| r.fill,
            |r| r.doc.set_fill_image(r.fill, Channel::Color, None),
        ),
        e(
            "set_fill_gradient",
            |r| r.fill,
            |r| {
                r.doc
                    .set_fill_gradient(r.fill, Channel::Height, Some(gradient()), false)
            },
        ),
        e(
            "set_fill_projection_image",
            |r| r.fill,
            |r| {
                r.doc.set_fill_projection(
                    r.fill,
                    Projection {
                        mode: ProjectionMode::Planar,
                        tiles: [2.0, 2.0],
                        ..Default::default()
                    },
                    false,
                )
            },
        ),
        e(
            "set_fill_projection_decal",
            |r| r.fill,
            |r| {
                r.doc.set_fill_image(r.fill, Channel::Color, None)?;
                r.doc.set_fill_projection(
                    r.fill,
                    Projection {
                        mode: ProjectionMode::Decal,
                        ..Default::default()
                    },
                    false,
                )
            },
        ),
        e(
            "set_fill_projection_plain",
            |r| r.plain_fill,
            |r| {
                r.doc.set_fill_projection(
                    r.plain_fill,
                    Projection {
                        mode: ProjectionMode::Planar,
                        tiles: [3.0, 3.0],
                        ..Default::default()
                    },
                    false,
                )
            },
        ),
        e(
            "set_fill_value_alpha",
            |r| r.fill,
            |r| {
                r.doc
                    .set_fill_value(r.fill, Channel::Color, Some(Rgba8::new(1, 2, 3, 77)), false)
            },
        ),
        e(
            "set_fill_value_colour",
            |r| r.fill,
            |r| {
                r.doc.set_fill_value(
                    r.fill,
                    Channel::Color,
                    Some(Rgba8::new(1, 2, 3, 200)),
                    false,
                )
            },
        ),
        e(
            "set_fill_value_clear",
            |r| r.fill,
            |r| r.doc.set_fill_value(r.fill, Channel::Color, None, false),
        ),
        e(
            "set_canvas_path",
            |r| r.path_layer,
            |r| r.doc.set_canvas_path(r.path_layer, canvas_path(2.0)),
        ),
        e(
            "set_path",
            |r| r.path_layer,
            |r| {
                let p = canvas_path(3.0);
                let drawn = rendered(&p);
                r.doc.set_path(r.path_layer, LayerPath::Canvas(p), drawn)
            },
        ),
        e(
            "rasterize",
            |r| r.path_layer,
            |r| r.doc.rasterize(r.path_layer),
        ),
        e(
            "set_channel_enabled_off",
            |r| r.base,
            |r| r.doc.set_channel_enabled(r.base, Channel::Height, false),
        ),
        e(
            "set_channel_enabled_noop",
            |r| r.base,
            |r| r.doc.set_channel_enabled(r.base, Channel::Height, true),
        ),
    ]
}
fn fx_lock(en: &Entry, lock: LayerLocks, grouped: bool) -> Vec<u8> {
    let mut r = rig();
    let mut ids = Ids::default();
    let target = (en.target)(&r);
    let holder = if grouped {
        r.doc.group_layers(&[target], "親").unwrap()
    } else {
        target
    };
    r.doc.set_layer_locks(holder, lock).unwrap();
    r.doc.clear_history().unwrap();
    let mut o = Vec::new();
    snapshot(&mut o, &r.doc, &mut ids);
    let result = (en.run)(&mut r);
    let ok = outcome(&mut o, &r.doc, result);
    after_op(&mut o, &mut r.doc, &mut ids, ok);
    o
}

// ───────── 手の書き込み × ロック × パスレイヤー ─────────

fn hard() -> BrushSettings {
    BrushSettings {
        radius: 3.0,
        hardness: 1.0,
        ..BrushSettings::default()
    }
}
fn two() -> [ChannelPaint; 2] {
    [
        ChannelPaint::new(Channel::Color, Rgba8::new(10, 200, 30, 255)),
        ChannelPaint::new(Channel::Emission, Rgba8::new(90, 90, 90, 255)),
    ]
}
const HAND_NAMES: [&str; 8] = [
    "begin_stroke",
    "begin_stroke_erase",
    "begin_material_stroke",
    "begin_triangle_fill",
    "fill",
    "fill_erase",
    "fill_material",
    "gradient_material",
];
fn hand_run(n: usize, d: &mut Document, id: LayerId) -> Result<(), CoreError> {
    let c = Rgba8::new(1, 2, 3, 255);
    match n {
        0 => d.begin_stroke(id, &hard()).map(|s| d.cancel_stroke(s)),
        1 => {
            let b = BrushSettings {
                erase: true,
                ..hard()
            };
            d.begin_stroke(id, &b).map(|s| d.cancel_stroke(s))
        }
        2 => d
            .begin_material_stroke(id, &two(), &hard())
            .map(|s| d.cancel_stroke(s)),
        3 => d
            .begin_triangle_fill(id, Channel::Color, c, 1., false)
            .map(|f| f.cancel(d)),
        4 => d.fill(id, Channel::Color, c, 1., None, false).map(|_| ()),
        5 => d.fill(id, Channel::Color, c, 1., None, true).map(|_| ()),
        6 => d.fill_material(id, &two(), 1., None, false).map(|_| ()),
        _ => {
            let ramp = GradientSettings {
                end: yolu_core::glam::DVec2::new(8., 6.),
                ..Default::default()
            };
            d.gradient_material(id, &two(), None, &ramp, None, false)
                .map(|_| ())
        }
    }
}
fn hand_lock(n: usize, lock: LayerLocks, grouped: bool, on_path: bool) -> Vec<u8> {
    let r = rig();
    let mut ids = Ids::default();
    let mut d = r.doc;
    let id = if on_path { r.path_layer } else { r.base };
    let holder = if grouped {
        d.group_layers(&[id], "親").unwrap()
    } else {
        id
    };
    d.set_layer_locks(holder, lock).unwrap();
    d.clear_history().unwrap();
    let mut o = Vec::new();
    snapshot(&mut o, &d, &mut ids);
    let result = hand_run(n, &mut d, id);
    outcome(&mut o, &d, result);
    u64b(&mut o, d.history_bytes());
    boolb(&mut o, d.has_active_stroke());
    snapshot(&mut o, &d, &mut ids);
    o
}
fn hand_transform(n: usize, lock: LayerLocks, grouped: bool) -> Vec<u8> {
    let r = rig();
    let mut ids = Ids::default();
    let mut d = r.doc;
    let holder = if grouped {
        d.group_layers(&[r.path_layer], "親").unwrap()
    } else {
        r.path_layer
    };
    d.set_layer_locks(holder, lock).unwrap();
    d.clear_history().unwrap();
    let mut o = Vec::new();
    snapshot(&mut o, &d, &mut ids);
    let result = if n == 0 {
        d.transform_layer(
            r.path_layer,
            Affine2D::translation(2., 1.),
            Resampling::Nearest,
            true,
        )
        .map(|_| ())
    } else {
        d.transform_layers(
            &[holder],
            Affine2D::translation(2., 1.),
            Resampling::Nearest,
        )
        .map(|_| ())
    };
    let ok = outcome(&mut o, &d, result);
    after_op(&mut o, &mut d, &mut ids, ok);
    o
}
fn group_path(lock: LayerLocks) -> Vec<u8> {
    let r = rig();
    let mut ids = Ids::default();
    let mut d = r.doc;
    let inner = d.add_layer("中身").unwrap();
    let group = d.group_layers(&[inner], "グループ").unwrap();
    d.set_layer_locks(group, lock).unwrap();
    d.clear_history().unwrap();
    let mut o = Vec::new();
    snapshot(&mut o, &d, &mut ids);
    let path = canvas_path(0.0);
    let drawn = rendered(&path);
    let result = d.add_path_layer("新", LayerPath::Canvas(path), drawn, Some(inner));
    let ok = outcome(&mut o, &d, result);
    after_op(&mut o, &mut d, &mut ids, ok);
    o
}

// ───────── レイヤーの操作と効果 ─────────

fn rotation() -> Affine2D {
    Affine2D::from_parts((20., 14.), (1., 0.), 17., (1.2, 0.8)).unwrap()
}
fn pair(d: &mut Document) -> (LayerId, LayerId) {
    let lower = d.add_layer("下").unwrap();
    paint(d, lower, Channel::Color, 11);
    let upper = d.add_layer("上").unwrap();
    for y in 5..8 {
        for x in 5..8 {
            d.set_channel_pixel(upper, Channel::Color, x, y, Rgba8::new(240, 30, 60, 255))
                .unwrap();
        }
    }
    (lower, upper)
}
fn blur(d: &mut Document, layer: LayerId, radius: u32) {
    d.add_filter(
        layer,
        FilterTarget::Content,
        FilterSpec::new(EffectSettings::blur(radius)).channels(&[Channel::Color]),
    )
    .unwrap();
}
fn anchor_stage(d: &mut Document, layer: LayerId) -> FilterId {
    d.add_filter(
        layer,
        FilterTarget::Content,
        FilterSpec::new(anchor_generator()).channels(&[Channel::Height]),
    )
    .unwrap()
}
fn reader_layer(d: &mut Document, anchor: AnchorId) -> LayerId {
    let layer = d.add_layer("読むレイヤー").unwrap();
    paint(d, layer, Channel::Color, 21);
    let reader = anchor_stage(d, layer);
    d.set_generator_anchor(
        layer,
        reader,
        Some(anchor),
        Channel::Height,
        ReadMode::Value,
        false,
    )
    .unwrap();
    layer
}

type Act = Box<dyn FnOnce(&mut Document) -> Result<Option<LayerMergeReport>, CoreError>>;

fn op(mut d: Document, act: Act) -> Vec<u8> {
    d.clear_history().unwrap();
    let mut o = Vec::new();
    let mut ids = Ids::default();
    snapshot(&mut o, &d, &mut ids);
    let result = act(&mut d);
    let (ok, rep) = match &result {
        Ok(r) => (true, r.clone()),
        Err(_) => (false, None),
    };
    outcome(&mut o, &d, result);
    if let Some(r) = rep {
        report(&mut o, &r);
    }
    after_op(&mut o, &mut d, &mut ids, ok);
    o
}
fn no_report<T>(r: Result<T, CoreError>) -> Result<Option<LayerMergeReport>, CoreError> {
    r.map(|_| None)
}
fn merged(r: Result<LayerMergeReport, CoreError>) -> Result<Option<LayerMergeReport>, CoreError> {
    r.map(Some)
}

fn resize_case(build: fn() -> Document, w: u32, h: u32, mode: CanvasResampling) -> Vec<u8> {
    let mut d = build();
    let mut o = Vec::new();
    let mut ids = Ids::default();
    snapshot(&mut o, &d, &mut ids);
    let before = o.clone();
    let result = d.resize_image(w, h, mode);
    let (ok, rep) = match &result {
        Ok(r) => (true, Some(r.clone())),
        Err(_) => (false, None),
    };
    outcome(&mut o, &d, result);
    if let Some(r) = rep {
        i32b(&mut o, r.notes.len() as i32);
        i32b(&mut o, r.surface_path_layers.len() as i32);
        for id in &r.surface_path_layers {
            i32b(&mut o, d.layer_index(*id).unwrap() as i32);
        }
        snapshot(&mut o, &d, &mut ids);
    }
    if ok {
        // 元の文書は変わらない（C# は別の文書を返す）。Rust は同じ文書の中で入れ替えるので、Undo で元へ戻ることを確かめる
        let mut after_undo = Vec::new();
        d.undo().unwrap();
        snapshot(&mut after_undo, &d, &mut ids);
        assert_eq!(after_undo, before, "大きさの変更の Undo で元の文書");
        d.redo().unwrap();
        o.extend(before);
    } else {
        snapshot(&mut o, &d, &mut ids);
    }
    o
}

fn world_with_blurs() -> Document {
    let mut r = rig();
    blur(&mut r.doc, r.base, 200);
    blur(&mut r.doc, r.base, 1);
    r.doc
        .add_filter(
            r.top,
            FilterTarget::Mask,
            FilterSpec::new(EffectSettings::blur(5)),
        )
        .unwrap();
    r.doc
}
fn world_with_many_blurs() -> Document {
    let mut r = rig();
    for _ in 0..3 {
        blur(&mut r.doc, r.base, 150);
    }
    r.doc
}
fn with_surface_path() -> Document {
    let mut r = rig();
    let d = &mut r.doc;
    let layer = d.add_layer("面のパス").unwrap();
    d.set_channel_enabled(layer, Channel::Height, true).unwrap();
    let mut scratch = Document::with_tile_size(W, H, 8).unwrap();
    let s = scratch.add_layer("s").unwrap();
    paint(&mut scratch, s, Channel::Height, 77);
    let surface = scratch
        .layer(s)
        .unwrap()
        .surface(Channel::Height)
        .unwrap()
        .clone();
    let path = SurfacePath {
        style: Default::default(),
        id: 9,
        channel: Channel::Height,
        brush: PathBrush(BrushSettings {
            radius: 0.1,
            ..path_brush().0
        }),
        points: vec![
            PathPoint::new(0, 0.2, 0.3, 1.0).unwrap(),
            PathPoint::new(1, 0.4, 0.2, 1.0).unwrap(),
        ],
        model_fingerprint: "synthetic-model".to_owned(),
        material: None,
    };
    d.set_path(
        layer,
        LayerPath::Surface(path),
        vec![(Channel::Height, surface)],
    )
    .unwrap();
    r.doc
}

type Case = (String, Box<dyn Fn() -> Vec<u8> + Send + Sync>);

fn cases() -> Vec<Case> {
    let mut v: Vec<Case> = Vec::new();
    let locks = [
        LayerLocks::NONE,
        LayerLocks::TRANSPARENCY,
        LayerLocks::PIXELS,
        LayerLocks::POSITION,
        LayerLocks::ALL,
    ];
    let group = |g: bool| if g { "group" } else { "layer" };
    let n_entries = entries().len();
    for i in 0..n_entries {
        for l in locks {
            for g in [false, true] {
                v.push((
                    format!("fxlock-{}-{}-{}", entries()[i].name, l.bits(), group(g)),
                    Box::new(move || fx_lock(&entries()[i], l, g)),
                ));
            }
        }
    }
    for (n, name) in HAND_NAMES.iter().enumerate() {
        for l in locks {
            for g in [false, true] {
                for on_path in [true, false] {
                    v.push((
                        format!(
                            "hand-{name}-{}-{}-{}",
                            l.bits(),
                            group(g),
                            if on_path { "path" } else { "raster" }
                        ),
                        Box::new(move || hand_lock(n, l, g, on_path)),
                    ));
                }
            }
        }
    }
    for n in 0..2 {
        for l in locks {
            for g in [false, true] {
                v.push((
                    format!(
                        "xform-{}-{}-{}",
                        if n == 0 { "layer" } else { "layers" },
                        l.bits(),
                        group(g)
                    ),
                    Box::new(move || hand_transform(n, l, g)),
                ));
            }
        }
    }
    for l in locks {
        v.push((
            format!("grouppath-{}", l.bits()),
            Box::new(move || group_path(l)),
        ));
    }
    let mut add =
        |name: &str, f: Box<dyn Fn() -> Vec<u8> + Send + Sync>| v.push((name.to_owned(), f));

    // 複製・削除・変形
    add(
        "dup-rig",
        Box::new(|| {
            let r = rig();
            let ids = [r.base, r.mid, r.top, r.path_layer];
            op(
                r.doc,
                Box::new(move |d| no_report(d.duplicate_layers(&ids))),
            )
        }),
    );
    add(
        "dup-group",
        Box::new(|| {
            let mut r = rig();
            let g = r.doc.group_layers(&[r.base, r.mid], "組").unwrap();
            op(
                r.doc,
                Box::new(move |d| no_report(d.duplicate_layers(&[g]))),
            )
        }),
    );
    add(
        "dup-reader-alone",
        Box::new(|| {
            let r = rig();
            let mid = r.mid;
            op(
                r.doc,
                Box::new(move |d| no_report(d.duplicate_layers(&[mid]))),
            )
        }),
    );
    for (name, pick) in [
        (
            "dup-single-reader",
            (|r: &Rig| r.mid) as fn(&Rig) -> LayerId,
        ),
        ("dup-single-base", |r: &Rig| r.base),
        ("dup-single-path", |r: &Rig| r.path_layer),
    ] {
        add(
            name,
            Box::new(move || {
                let r = rig();
                let id = pick(&r);
                op(
                    r.doc,
                    Box::new(move |d| no_report(d.duplicate_layer(id, None))),
                )
            }),
        );
    }
    add(
        "remove-base",
        Box::new(|| {
            let r = rig();
            let base = r.base;
            op(
                r.doc,
                Box::new(move |d| no_report(d.remove_layers(&[base]))),
            )
        }),
    );
    add(
        "remove-group",
        Box::new(|| {
            let mut r = rig();
            let g = r.doc.group_layers(&[r.base, r.mid], "組").unwrap();
            op(r.doc, Box::new(move |d| no_report(d.remove_layers(&[g]))))
        }),
    );
    add(
        "transform-translate",
        Box::new(|| {
            let r = rig();
            let ids = [r.base, r.top];
            op(
                r.doc,
                Box::new(move |d| {
                    no_report(d.transform_layers(
                        &ids,
                        Affine2D::translation(3., -2.),
                        Resampling::Bilinear,
                    ))
                }),
            )
        }),
    );
    add(
        "transform-rotate",
        Box::new(|| {
            let r = rig();
            let ids = [r.base, r.top];
            op(
                r.doc,
                Box::new(move |d| {
                    no_report(d.transform_layers(&ids, rotation(), Resampling::Bilinear))
                }),
            )
        }),
    );
    add(
        "transform-with-path",
        Box::new(|| {
            let r = rig();
            let ids = [r.base, r.path_layer];
            op(
                r.doc,
                Box::new(move |d| {
                    no_report(d.transform_layers(&ids, rotation(), Resampling::Bilinear))
                }),
            )
        }),
    );
    add(
        "transform-path-group",
        Box::new(|| {
            let mut r = rig();
            let g = r.doc.group_layers(&[r.path_layer], "組").unwrap();
            op(
                r.doc,
                Box::new(move |d| {
                    no_report(d.transform_layers(&[g], rotation(), Resampling::Bilinear))
                }),
            )
        }),
    );
    add(
        "transform-single-filtered",
        Box::new(|| {
            let r = rig();
            let base = r.base;
            op(
                r.doc,
                Box::new(move |d| {
                    no_report(d.transform_layer(base, rotation(), Resampling::Bilinear, true))
                }),
            )
        }),
    );
    add(
        "transform-single-path",
        Box::new(|| {
            let r = rig();
            let p = r.path_layer;
            op(
                r.doc,
                Box::new(move |d| {
                    no_report(d.transform_layer(p, rotation(), Resampling::Bilinear, true))
                }),
            )
        }),
    );

    // パスで描かれたチャンネルを無効にする: 組を持たないパスは断る。組を持つパスは基準のチャンネルも組のチャンネルも無効にできる
    for (name, material, channel) in [
        ("channel-off-path-plain", false, Channel::Color),
        ("channel-off-path-material-base", true, Channel::Color),
        ("channel-off-path-material-other", true, Channel::Emission),
    ] {
        add(
            name,
            Box::new(move || {
                let mut r = rig();
                let p = r.path_layer;
                if material {
                    let mut path = canvas_path(0.0);
                    path.material = Some(vec![
                        yolu_core::paths::ChannelPaint {
                            channel: Channel::Color,
                            color: Rgba8::new(10, 200, 30, 255),
                        },
                        yolu_core::paths::ChannelPaint {
                            channel: Channel::Emission,
                            color: Rgba8::new(90, 90, 90, 255),
                        },
                    ]);
                    r.doc.set_canvas_path(p, path).unwrap();
                }
                op(
                    r.doc,
                    Box::new(move |d| no_report(d.set_channel_enabled(p, channel, false))),
                )
            }),
        );
    }

    // 並べ替え・表示・不透明度・クリッピング・選択範囲の変形（効果を持つレイヤー・Anchor を読むレイヤーとの組み合わせ）
    add(
        "move-reader-below",
        Box::new(|| {
            let r = rig();
            let mid = r.mid;
            op(
                r.doc,
                Box::new(move |d| no_report(d.move_layers(&[mid], None, 0))),
            )
        }),
    );
    add(
        "move-reader-into-group",
        Box::new(|| {
            let mut r = rig();
            let g = r.doc.group_layers(&[r.top], "組").unwrap();
            let mid = r.mid;
            op(
                r.doc,
                Box::new(move |d| no_report(d.move_layers(&[mid], Some(g), 0))),
            )
        }),
    );
    add(
        "step-base-up",
        Box::new(|| {
            let r = rig();
            let base = r.base;
            op(
                r.doc,
                Box::new(move |d| no_report(d.step_layers(&[base], true))),
            )
        }),
    );
    add(
        "hide-base",
        Box::new(|| {
            let r = rig();
            let base = r.base;
            op(
                r.doc,
                Box::new(move |d| no_report(d.set_layer_visible(base, false))),
            )
        }),
    );
    add(
        "opacity-base",
        Box::new(|| {
            let r = rig();
            let base = r.base;
            op(
                r.doc,
                Box::new(move |d| no_report(d.set_layer_opacity(base, 0.5, false))),
            )
        }),
    );
    add(
        "clip-reader",
        Box::new(|| {
            let r = rig();
            let mid = r.mid;
            op(
                r.doc,
                Box::new(move |d| no_report(d.set_layer_clipping(mid, true))),
            )
        }),
    );
    add(
        "region-filtered",
        Box::new(|| {
            let r = rig();
            let base = r.base;
            let region = SelectionMask::rectangle(&r.doc, 5, 1, 16, 8);
            op(
                r.doc,
                Box::new(move |d| {
                    no_report(d.transform_layer_region(
                        base,
                        rotation(),
                        Resampling::Bilinear,
                        true,
                        &region,
                    ))
                }),
            )
        }),
    );
    add(
        "region-path",
        Box::new(|| {
            let r = rig();
            let p = r.path_layer;
            let region = SelectionMask::rectangle(&r.doc, 5, 1, 16, 8);
            op(
                r.doc,
                Box::new(move |d| {
                    no_report(d.transform_layer_region(
                        p,
                        rotation(),
                        Resampling::Bilinear,
                        true,
                        &region,
                    ))
                }),
            )
        }),
    );

    // 結合
    add(
        "merge-down-clipped-blur",
        Box::new(|| {
            let mut d = new_doc();
            let (lower, upper) = pair(&mut d);
            blur(&mut d, upper, 3);
            blur(&mut d, lower, 2);
            d.set_layer_clipping(upper, true).unwrap();
            op(d, Box::new(move |d| merged(d.merge_down(upper, 255))))
        }),
    );
    add(
        "merge-down-blur",
        Box::new(|| {
            let mut d = new_doc();
            let (_, upper) = pair(&mut d);
            blur(&mut d, upper, 6);
            op(d, Box::new(move |d| merged(d.merge_down(upper, 2))))
        }),
    );
    add(
        "merge-down-isolated-mask",
        Box::new(|| {
            let mut d = new_doc();
            let (lower, upper) = pair(&mut d);
            d.add_layer_mask(lower).unwrap();
            for y in 0..H {
                for x in 0..W {
                    if (x + y) % 3 == 0 {
                        d.set_mask_pixel(lower, x, y, 140).unwrap();
                    }
                }
            }
            d.add_filter(
                lower,
                FilterTarget::Mask,
                FilterSpec::new(EffectSettings::blur(2)),
            )
            .unwrap();
            d.add_filter(
                upper,
                FilterTarget::Content,
                FilterSpec::new(EffectSettings::levels(0.1, 0.9, 1.2, 0.0, 1.0))
                    .channels(&[Channel::Color]),
            )
            .unwrap();
            d.set_layer_opacity(lower, 0.5, false).unwrap();
            op(d, Box::new(move |d| merged(d.merge_down(upper, 0))))
        }),
    );
    add(
        "merge-down-kept-mask",
        Box::new(|| {
            let mut d = new_doc();
            let (lower, upper) = pair(&mut d);
            d.add_layer_mask(lower).unwrap();
            for y in 0..H {
                for x in 28..W {
                    d.set_mask_pixel(lower, x, y, 200).unwrap();
                }
            }
            d.add_filter(
                lower,
                FilterTarget::Mask,
                FilterSpec::new(EffectSettings::blur(2)),
            )
            .unwrap();
            let below = d.add_layer("一番下").unwrap();
            d.set_channel_pixel(below, Channel::Color, 3, 3, Rgba8::new(1, 2, 3, 255))
                .unwrap();
            d.move_layer(below, 0).unwrap();
            op(d, Box::new(move |d| merged(d.merge_down(upper, 2))))
        }),
    );
    add(
        "merge-down-anchors",
        Box::new(|| {
            let mut d = new_doc();
            let (lower, upper) = pair(&mut d);
            blur(&mut d, upper, 2);
            d.add_anchor(lower, AnchorPlacement::Layer, Some("下"), None)
                .unwrap();
            let upper_anchor = d
                .add_anchor(upper, AnchorPlacement::Layer, Some("上"), None)
                .unwrap();
            reader_layer(&mut d, upper_anchor);
            op(d, Box::new(move |d| merged(d.merge_down(upper, 2))))
        }),
    );
    add(
        "merge-down-lower-anchor-read",
        Box::new(|| {
            let mut d = new_doc();
            let (lower, upper) = pair(&mut d);
            blur(&mut d, upper, 2);
            let lower_anchor = d
                .add_anchor(lower, AnchorPlacement::Layer, Some("下"), None)
                .unwrap();
            reader_layer(&mut d, lower_anchor);
            op(d, Box::new(move |d| merged(d.merge_down(upper, 2))))
        }),
    );
    add(
        "merge-visible-generator",
        Box::new(|| {
            let mut r = rig();
            let layer = r.doc.add_layer("空のレイヤー").unwrap();
            let f = anchor_stage(&mut r.doc, layer);
            r.doc
                .set_generator_anchor(
                    layer,
                    f,
                    Some(r.anchor),
                    Channel::Height,
                    ReadMode::Value,
                    false,
                )
                .unwrap();
            op(r.doc, Box::new(|d| merged(d.merge_visible("merged", 2))))
        }),
    );
    add(
        "merge-visible-rig",
        Box::new(|| {
            op(
                rig().doc,
                Box::new(|d| merged(d.merge_visible("merged", 255))),
            )
        }),
    );
    add(
        "merge-down-path-onto-base",
        Box::new(|| {
            let mut r = rig();
            // 土台の Anchor を読む段は外しておく（結合で土台の Anchor が無くなると読む段は入力のまま通すようになり、見た目が変わる。
            // C# の結合の報告は、その変化を数えそこなう: 派生の Anchor のキャッシュが古い。Rust は数える。seam_ops.rs で確かめる）
            r.doc.remove_filter(r.mid, r.reader).unwrap();
            r.doc.move_layer(r.path_layer, 1).unwrap();
            let p = r.path_layer;
            op(r.doc, Box::new(move |d| merged(d.merge_down(p, 255))))
        }),
    );
    add(
        "merge-group-path",
        Box::new(|| {
            let mut r = rig();
            let g = r
                .doc
                .group_layers(&[r.base, r.path_layer], "グループ")
                .unwrap();
            r.doc
                .add_anchor(g, AnchorPlacement::Layer, Some("グループ"), None)
                .unwrap();
            op(r.doc, Box::new(move |d| merged(d.merge_group(g, 255))))
        }),
    );
    add(
        "merge-layers-path",
        Box::new(|| {
            let mut r = rig();
            r.doc.remove_filter(r.mid, r.reader).unwrap();
            let ids = [r.base, r.path_layer];
            op(r.doc, Box::new(move |d| merged(d.merge_layers(&ids, 255))))
        }),
    );
    add(
        "merge-layers-mask-filter",
        Box::new(|| {
            let r = rig();
            let ids = [r.mid, r.top];
            op(r.doc, Box::new(move |d| merged(d.merge_layers(&ids, 255))))
        }),
    );

    // 効いていない効果は焼かない
    add(
        "merge-inactive-no-anchor",
        Box::new(|| {
            let mut d = new_doc();
            let (_, upper) = pair(&mut d);
            anchor_stage(&mut d, upper);
            op(d, Box::new(move |d| merged(d.merge_down(upper, 255))))
        }),
    );
    add(
        "merge-inactive-lower-content",
        Box::new(|| {
            let mut d = new_doc();
            let (lower, upper) = pair(&mut d);
            anchor_stage(&mut d, lower);
            op(d, Box::new(move |d| merged(d.merge_down(upper, 255))))
        }),
    );
    // 下のレイヤーの下に見えるレイヤーを置いて、分離の結合にしない（マスクが効果ごと結果に残る結合）。分離の結合は下のマスクのフィルターも焼くが、
    // C# はそのマスクの効いていない Generator を見ずに黙って落とす。Rust は意図して断る（seam_ops.rs の
    // an_isolated_merge_refuses_an_inactive_generator_in_the_lower_mask で確かめる）
    add(
        "merge-inactive-lower-mask",
        Box::new(|| {
            let mut d = new_doc();
            let (lower, upper) = pair(&mut d);
            let below = d.add_layer("下の下").unwrap();
            paint(&mut d, below, Channel::Color, 12);
            d.move_layers(&[below], None, 0).unwrap();
            d.add_layer_mask(lower).unwrap();
            d.add_filter(
                lower,
                FilterTarget::Mask,
                FilterSpec::new(anchor_generator()),
            )
            .unwrap();
            op(d, Box::new(move |d| merged(d.merge_down(upper, 255))))
        }),
    );
    add(
        "merge-inactive-needs-map",
        Box::new(|| {
            let mut d = new_doc();
            let (_, upper) = pair(&mut d);
            d.add_filter(
                upper,
                FilterTarget::Content,
                FilterSpec::new(EffectSettings::generator(gradient())).channels(&[Channel::Color]),
            )
            .unwrap();
            op(d, Box::new(move |d| merged(d.merge_down(upper, 255))))
        }),
    );
    add(
        "merge-inactive-disabled-passes",
        Box::new(|| {
            let mut d = new_doc();
            let (_, upper) = pair(&mut d);
            d.add_filter(
                upper,
                FilterTarget::Content,
                FilterSpec::new(anchor_generator())
                    .channels(&[Channel::Height])
                    .disabled(),
            )
            .unwrap();
            op(d, Box::new(move |d| merged(d.merge_down(upper, 255))))
        }),
    );
    add(
        "merge-inactive-visible",
        Box::new(|| {
            let mut d = new_doc();
            let (_, upper) = pair(&mut d);
            anchor_stage(&mut d, upper);
            op(d, Box::new(|d| merged(d.merge_visible("merged", 255))))
        }),
    );
    add(
        "merge-inactive-group",
        Box::new(|| {
            let mut d = new_doc();
            let (lower, upper) = pair(&mut d);
            anchor_stage(&mut d, upper);
            let g = d.group_layers(&[lower, upper], "組").unwrap();
            op(d, Box::new(move |d| merged(d.merge_group(g, 255))))
        }),
    );
    add(
        "merge-inactive-layers",
        Box::new(|| {
            let mut d = new_doc();
            let (lower, upper) = pair(&mut d);
            anchor_stage(&mut d, upper);
            op(
                d,
                Box::new(move |d| merged(d.merge_layers(&[lower, upper], 255))),
            )
        }),
    );
    add(
        "merge-inactive-then-lock",
        Box::new(|| {
            let mut d = new_doc();
            let (lower, upper) = pair(&mut d);
            anchor_stage(&mut d, upper);
            d.set_layer_locks(lower, LayerLocks::PIXELS).unwrap();
            op(d, Box::new(move |d| merged(d.merge_down(upper, 255))))
        }),
    );

    // 画像の大きさ
    add(
        "resize-blurs-up",
        Box::new(|| resize_case(world_with_blurs, 80, 56, CanvasResampling::Bilinear)),
    );
    add(
        "resize-blurs-down",
        Box::new(|| resize_case(world_with_blurs, 10, 7, CanvasResampling::Area)),
    );
    add(
        "resize-blurs-nearest",
        Box::new(|| resize_case(world_with_blurs, 61, 43, CanvasResampling::Nearest)),
    );
    add(
        "resize-too-far",
        Box::new(|| resize_case(world_with_many_blurs, 60, 42, CanvasResampling::Nearest)),
    );
    add(
        "resize-too-far-down",
        Box::new(|| resize_case(world_with_many_blurs, 30, 21, CanvasResampling::Nearest)),
    );
    add(
        "resize-rig-up",
        Box::new(|| resize_case(|| rig().doc, 80, 56, CanvasResampling::Bilinear)),
    );
    add(
        "resize-rig-odd",
        Box::new(|| resize_case(|| rig().doc, 53, 31, CanvasResampling::Area)),
    );
    add(
        "resize-rig-down",
        Box::new(|| resize_case(|| rig().doc, 20, 14, CanvasResampling::Bilinear)),
    );
    add(
        "resize-surface-path",
        Box::new(|| resize_case(with_surface_path, 80, 56, CanvasResampling::Bilinear)),
    );
    add(
        "resize-locked",
        Box::new(|| {
            resize_case(
                || {
                    let mut r = rig();
                    r.doc.set_layer_locks(r.base, LayerLocks::ALL).unwrap();
                    r.doc
                        .set_layer_locks(r.path_layer, LayerLocks::PIXELS)
                        .unwrap();
                    r.doc
                },
                60,
                42,
                CanvasResampling::Bilinear,
            )
        }),
    );
    v
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// 実 C# の PaintDocument が出した 事例（SeamGolden.cs）と、Rust が同じ台本で出したバイト列の SHA-256 が全部同じ。
#[test]
fn csharp_seam_golden_matches_every_case() {
    let golden: HashMap<&str, &str> = include_str!("../golden/seam.txt")
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .map(|l| l.split_once(' ').unwrap())
        .collect();
    let cases = cases();
    assert_eq!(cases.len(), golden.len(), "事例の数");
    let dump = std::env::var("SEAM_DUMP_DIR").ok();
    let only = std::env::var("SEAM_ONLY").ok();
    let mut bad = Vec::new();
    for degree in [1, 4] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(degree)
            .build()
            .unwrap();
        pool.install(|| {
            for (name, run) in &cases {
                if only.as_ref().is_some_and(|o| !name.contains(o.as_str())) {
                    continue;
                }
                let bytes = run();
                let want = golden
                    .get(name.as_str())
                    .unwrap_or_else(|| panic!("正解に無い事例: {name}"));
                let got = hex(&Sha256::digest(&bytes));
                if got != *want && golden_update::updating() {
                    if degree == 1 {
                        let path = golden_update::tests_dir().join("golden/seam.txt");
                        golden_update::replace_line(&path, name, &format!("{name} {got}"));
                    }
                    continue;
                }
                if got != *want {
                    if let Some(dir) = &dump {
                        std::fs::write(std::path::Path::new(dir).join(name), &bytes).unwrap();
                    }
                    bad.push(format!("{name}（並列度 {degree}）"));
                }
            }
        });
    }
    assert!(
        bad.is_empty(),
        "C# と食い違った事例 {} 件: {:?}",
        bad.len(),
        &bad[..bad.len().min(40)]
    );
}
