//! ロックと効果・パスのつなぎ目: 検査の順（ロック → パスの層）と、効果の設定の変更がどのロックで断られるかを、C# の
//! PaintDocument と同じに保つ。断ったら何も変えず、ロックを外せば同じ操作が通る。
//! C# との照合は `seam_golden.rs`（実 C# が出した断り方）。ここは Rust だけで確かめられる性質（何も変わらない・外せば通る・親のグループ）。
use crate::attach_support;
use crate::seam_support;
use attach_support::*;
use seam_support::*;
use yolu_core::fill_image::{Projection, ProjectionMode};
use yolu_core::generator::{self, anchor::ReadMode, Settings};
use yolu_core::material::{ChannelPaint, GradientSettings};
use yolu_core::{
    Affine2D, AnchorPlacement, BrushSettings, Channel, CoreError, Document, EffectSettings,
    FilterSpec, FilterTarget, LayerId, LayerLocks, LayerPath, Resampling, Rgba8,
};

type Op = Box<dyn Fn(&mut Rig) -> Result<(), CoreError>>;

/// 設定を変える入口 1 つ。`target` はロックを掛ける層、`needs_pixels` は画像のロックでも断るか、`needs_transparency` は
/// 透明部分のロックでも断るか（どちらも C# の入口に合わせた表）。
struct Entry {
    name: &'static str,
    target: fn(&Rig) -> LayerId,
    needs_pixels: bool,
    needs_transparency: bool,
    /// すべてのロックでも断らない入口（Anchor の名前の変更）。
    never: bool,
    run: Op,
}

fn entry(
    name: &'static str,
    target: fn(&Rig) -> LayerId,
    needs_pixels: bool,
    needs_transparency: bool,
    run: impl Fn(&mut Rig) -> Result<(), CoreError> + 'static,
) -> Entry {
    Entry {
        name,
        target,
        needs_pixels,
        needs_transparency,
        never: name.starts_with("rename_anchor"),
        run: Box::new(run),
    }
}

fn gradient() -> Settings {
    let mut g = Settings::new(generator::Kind::ShapeGradient);
    g.ramp = Some(generator::Ramp::default());
    g.blend = generator::Blend::Replace;
    g
}

fn entries() -> Vec<Entry> {
    vec![
        // フィルター: 非破壊（画素は変えない）なので、すべてのロックだけで断る
        entry(
            "add_filter",
            |r| r.base,
            false,
            false,
            |r| {
                r.doc
                    .add_filter(
                        r.base,
                        FilterTarget::Content,
                        FilterSpec::new(EffectSettings::invert()).channels(&[Channel::Height]),
                    )
                    .map(|_| ())
            },
        ),
        entry(
            "remove_filter",
            |r| r.base,
            false,
            false,
            |r| r.doc.remove_filter(r.base, r.blur),
        ),
        entry(
            "set_filter_enabled",
            |r| r.base,
            false,
            false,
            |r| r.doc.set_filter_enabled(r.base, r.blur, false),
        ),
        entry(
            "set_filter_strength",
            |r| r.base,
            false,
            false,
            |r| r.doc.set_filter_strength(r.base, r.blur, 0.5, false),
        ),
        entry(
            "set_filter_settings",
            |r| r.base,
            false,
            false,
            |r| {
                r.doc
                    .set_filter_settings(r.base, r.blur, EffectSettings::blur(7), false)
            },
        ),
        entry(
            "set_filter_channels",
            |r| r.base,
            false,
            false,
            |r| {
                r.doc
                    .set_filter_channels(r.base, r.blur, &[Channel::Color, Channel::Height])
            },
        ),
        entry(
            "move_filter（先頭へ）",
            |r| r.base,
            false,
            false,
            |r| {
                r.doc
                    .add_filter(
                        r.base,
                        FilterTarget::Content,
                        FilterSpec::new(EffectSettings::invert()).channels(&[Channel::Height]),
                    )
                    .map(|_| ())?;
                r.doc.move_filter(r.base, r.blur, 1)
            },
        ),
        entry(
            "マスクの add_filter",
            |r| r.top,
            false,
            false,
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
        entry(
            "マスクの set_filter_strength",
            |r| r.top,
            false,
            false,
            |r| r.doc.set_filter_strength(r.top, r.mask_blur, 0.4, false),
        ),
        // Anchor: 名前の変更だけは断らない
        entry(
            "add_anchor",
            |r| r.mid,
            false,
            false,
            |r| {
                r.doc
                    .add_anchor(r.mid, AnchorPlacement::Layer, None, None)
                    .map(|_| ())
            },
        ),
        entry(
            "remove_anchor",
            |r| r.base,
            false,
            false,
            |r| r.doc.remove_anchor(r.anchor),
        ),
        entry(
            "rename_anchor（断らない）",
            |r| r.base,
            false,
            false,
            |r| r.doc.rename_anchor(r.anchor, "別の名前"),
        ),
        entry(
            "set_generator_anchor",
            |r| r.mid,
            false,
            false,
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
        // 塗りつぶし: 画像・グラデーションは画素を決め直す（アルファも変わる）ので、画像・透明部分のロックでも断る
        entry(
            "set_fill_image（外す）",
            |r| r.fill,
            true,
            true,
            |r| r.doc.set_fill_image(r.fill, Channel::Color, None),
        ),
        entry(
            "set_fill_gradient",
            |r| r.fill,
            true,
            true,
            |r| {
                r.doc
                    .set_fill_gradient(r.fill, Channel::Height, Some(gradient()), false)
            },
        ),
        // 投影: 画像のある層は画素を変える
        entry(
            "set_fill_projection（画像のある層）",
            |r| r.fill,
            true,
            true,
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
        entry(
            "set_fill_projection（デカールへ）",
            |r| r.fill,
            true,
            true,
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
        // 画像もデカールも無い層の投影は画素を変えない（すべてのロックだけ）
        entry(
            "set_fill_projection（素の層）",
            |r| r.plain_fill,
            false,
            false,
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
        // 値: アルファが変わると透明部分のロックでも断る。色だけなら画像のロックまで
        entry(
            "set_fill_value（アルファを変える）",
            |r| r.fill,
            true,
            true,
            |r| {
                r.doc
                    .set_fill_value(r.fill, Channel::Color, Some(Rgba8::new(1, 2, 3, 77)), false)
            },
        ),
        entry(
            "set_fill_value（色だけ）",
            |r| r.fill,
            true,
            false,
            |r| {
                r.doc.set_fill_value(
                    r.fill,
                    Channel::Color,
                    Some(Rgba8::new(1, 2, 3, 200)),
                    false,
                )
            },
        ),
        entry(
            "set_fill_value（画像のあるチャンネルの値を消す）",
            |r| r.fill,
            true,
            true,
            |r| r.doc.set_fill_value(r.fill, Channel::Color, None, false),
        ),
        // パス: 画素を描き直す（アルファも変わる）ので、画像・透明部分のロックでも断る。外す（rasterize）のはすべてのロックだけ
        entry(
            "set_canvas_path",
            |r| r.path_layer,
            true,
            true,
            |r| r.doc.set_canvas_path(r.path_layer, path_points(2.0)),
        ),
        entry(
            "set_path",
            |r| r.path_layer,
            true,
            true,
            |r| {
                let p = path_points(3.0);
                let drawn = rendered(&p);
                r.doc.set_path(r.path_layer, LayerPath::Canvas(p), drawn)
            },
        ),
        entry(
            "rasterize",
            |r| r.path_layer,
            false,
            false,
            |r| r.doc.rasterize(r.path_layer),
        ),
    ]
}

fn expected(e: &Entry, lock: LayerLocks) -> Option<LayerLocks> {
    if e.never {
        None
    } else if lock == LayerLocks::ALL {
        Some(LayerLocks::ALL)
    } else if lock == LayerLocks::PIXELS && e.needs_pixels {
        Some(LayerLocks::PIXELS)
    } else if lock == LayerLocks::TRANSPARENCY && e.needs_transparency {
        Some(LayerLocks::TRANSPARENCY)
    } else {
        None
    }
}

fn fingerprint(r: &Rig) -> (u64, usize, Vec<u8>, Vec<u8>) {
    (
        r.doc.revision(),
        r.doc.undo_count(),
        whole(&r.doc, Channel::Color),
        whole(&r.doc, Channel::Height),
    )
}

/// 効果・パスの設定を変える入口が、ロックの種類ごとに C# と同じ表のとおり断り、断ったあとは何も変えない。ロックを外せば同じ入口が通る。
/// ロックは層自身にも親のグループにも掛ける（持ち主はグループ）。
#[test]
fn effect_setters_obey_the_layer_locks_like_csharp() {
    let (mut refused, mut passed) = (0, 0);
    for lock in [
        LayerLocks::NONE,
        LayerLocks::TRANSPARENCY,
        LayerLocks::PIXELS,
        LayerLocks::POSITION,
        LayerLocks::ALL,
    ] {
        for e in &entries() {
            for grouped in [false, true] {
                let mut r = rig();
                let target = (e.target)(&r);
                let holder = if grouped {
                    r.doc.group_layers(&[target], "親").unwrap()
                } else {
                    target
                };
                r.doc.set_layer_locks(holder, lock).unwrap();
                r.doc.clear_history().unwrap();
                let before = fingerprint(&r);
                let result = (e.run)(&mut r);
                let context = format!("{} / {lock:?} / grouped={grouped}", e.name);
                match expected(e, lock) {
                    Some(named) => {
                        assert_eq!(
                            result,
                            Err(CoreError::LayerLocked {
                                layer: target,
                                holder,
                                lock: named
                            }),
                            "{context}"
                        );
                        assert_eq!(fingerprint(&r), before, "{context}: 断ったら何も変えない");
                        r.doc.set_layer_locks(holder, LayerLocks::NONE).unwrap();
                        assert!((e.run)(&mut r).is_ok(), "{context}: 外せば通る");
                        refused += 1;
                    }
                    None => {
                        assert_eq!(result, Ok(()), "{context}");
                        passed += 1;
                    }
                }
            }
        }
    }
    // 入口の数 × ロック 5 種 × 層/親グループ 2。断る数は表から数えた値で固定する（表が崩れて数がずれたら落ちる）
    assert_eq!(entries().len(), 24);
    // すべてのロック: 23 入口（Anchor の名前の変更を除く）、画像のロック: 9 入口、透明部分のロック: 8 入口（それぞれ 層 / 親グループの 2 通り）
    assert_eq!(
        (refused, passed),
        (2 * (23 + 9 + 8), 24 * 5 * 2 - 2 * (23 + 9 + 8))
    );
}

/// 新しい層が親のグループのロックで断られる（パスの層を、ロックされたグループの中へ足す）。層も作らず、外せば足せる。
#[test]
fn a_path_layer_cannot_be_added_into_a_locked_group() {
    for lock in [
        LayerLocks::TRANSPARENCY,
        LayerLocks::PIXELS,
        LayerLocks::ALL,
        LayerLocks::POSITION,
    ] {
        let mut r = rig();
        let inner = r.doc.add_layer("中身").unwrap();
        let group = r.doc.group_layers(&[inner], "グループ").unwrap();
        r.doc.set_layer_locks(group, lock).unwrap();
        r.doc.clear_history().unwrap();
        let count = r.doc.layers().len();
        let revision = r.doc.revision();
        let path = path_points(0.0);
        let result = r.doc.add_path_layer(
            "新",
            LayerPath::Canvas(path.clone()),
            rendered(&path),
            Some(inner),
        );
        if lock == LayerLocks::POSITION {
            result.unwrap();
            continue;
        }
        match result {
            Err(CoreError::LayerLocked {
                holder,
                lock: named,
                ..
            }) => {
                assert_eq!(holder, group, "{lock:?}");
                assert_eq!(named, lock, "{lock:?}");
            }
            other => panic!("{lock:?}: {other:?}"),
        }
        assert_eq!(r.doc.layers().len(), count, "{lock:?}: 層を作らない");
        assert_eq!(r.doc.revision(), revision);
        assert_eq!(r.doc.undo_count(), 0);
        r.doc.set_layer_locks(group, LayerLocks::NONE).unwrap();
        r.doc
            .add_path_layer(
                "新",
                LayerPath::Canvas(path.clone()),
                rendered(&path),
                Some(inner),
            )
            .unwrap();
    }
}

type Write = Box<dyn Fn(&mut Document, LayerId) -> Result<(), CoreError>>;

fn p_entry(
    name: &'static str,
    by_pixels: bool,
    erases: bool,
    run: impl Fn(&mut Document, LayerId) -> Result<(), CoreError> + 'static,
) -> (&'static str, bool, bool, Write) {
    (name, by_pixels, erases, Box::new(run))
}

fn two() -> [ChannelPaint; 2] {
    [
        ChannelPaint::new(Channel::Color, Rgba8::new(10, 200, 30, 255)),
        ChannelPaint::new(Channel::Emission, Rgba8::new(90, 90, 90, 255)),
    ]
}

fn hard() -> BrushSettings {
    BrushSettings {
        radius: 3.0,
        hardness: 1.0,
        ..BrushSettings::default()
    }
}

/// パスで描かれた層への手の書き込みは、先にロックで断り（C# の RefuseLockedPixels → RefusePathLayer）、ロックが通ればパスの層として断る。
/// 動かす（transform）だけは、型・パスの層の検査がロックより先（C# の RequireTransformable → RefuseLockedTransform）で、ロックに
/// 関わらずパスの層として断る。どれも断ったら何も変えない。
#[test]
fn hand_writes_to_a_path_layer_check_the_locks_first_and_transforms_check_the_path_first() {
    let path_error = CoreError::Unsupported("パスで描かれた層には手で描けない");
    let writes: Vec<(&str, bool, bool, Write)> = vec![
        p_entry("begin_stroke", true, false, |d, id| {
            d.begin_stroke(id, &hard()).map(|s| d.cancel_stroke(s))
        }),
        p_entry("begin_stroke（消す）", true, true, |d, id| {
            let b = BrushSettings {
                erase: true,
                ..hard()
            };
            d.begin_stroke(id, &b).map(|s| d.cancel_stroke(s))
        }),
        p_entry("begin_material_stroke", true, false, |d, id| {
            d.begin_material_stroke(id, &two(), &hard())
                .map(|s| d.cancel_stroke(s))
        }),
        p_entry("begin_triangle_fill", true, false, |d, id| {
            d.begin_triangle_fill(id, Channel::Color, Rgba8::new(1, 2, 3, 255), 1., false)
                .map(|f| f.cancel(d))
        }),
        p_entry("fill", true, false, |d, id| {
            d.fill(
                id,
                Channel::Color,
                Rgba8::new(1, 2, 3, 255),
                1.,
                None,
                false,
            )
            .map(|_| ())
        }),
        p_entry("fill（消す）", true, true, |d, id| {
            d.fill(id, Channel::Color, Rgba8::new(1, 2, 3, 255), 1., None, true)
                .map(|_| ())
        }),
        p_entry("fill_material", true, false, |d, id| {
            d.fill_material(id, &two(), 1., None, false).map(|_| ())
        }),
        p_entry("gradient_material", true, false, |d, id| {
            let ramp = GradientSettings {
                end: yolu_core::glam::DVec2::new(8., 6.),
                ..Default::default()
            };
            d.gradient_material(id, &two(), None, &ramp, None, false)
                .map(|_| ())
        }),
    ];
    for lock in [
        LayerLocks::NONE,
        LayerLocks::TRANSPARENCY,
        LayerLocks::PIXELS,
        LayerLocks::ALL,
    ] {
        for (name, by_pixels, erases, run) in &writes {
            for grouped in [false, true] {
                let r = rig();
                let (mut doc, id) = (r.doc, r.path_layer);
                let holder = if grouped {
                    doc.group_layers(&[id], "親").unwrap()
                } else {
                    id
                };
                doc.set_layer_locks(holder, lock).unwrap();
                doc.clear_history().unwrap();
                let (revision, pixels) = (doc.revision(), whole(&doc, Channel::Color));
                let result = run(&mut doc, id);
                let by_lock = lock == LayerLocks::ALL
                    || lock == LayerLocks::PIXELS && *by_pixels
                    || lock == LayerLocks::TRANSPARENCY && *erases;
                let want = if by_lock {
                    CoreError::LayerLocked {
                        layer: id,
                        holder,
                        lock,
                    }
                } else {
                    path_error.clone()
                };
                assert_eq!(result, Err(want), "{name} / {lock:?} / grouped={grouped}");
                assert_eq!(doc.revision(), revision, "{name}");
                assert_eq!(doc.undo_count(), 0, "{name}");
                assert_eq!(whole(&doc, Channel::Color), pixels, "{name}");
                assert!(!doc.has_active_stroke(), "{name}");
            }
        }
        // 動かす: ロックに関わらずパスの層の断り（型・パスの検査が先）
        for grouped in [false, true] {
            let r = rig();
            let (mut doc, id) = (r.doc, r.path_layer);
            let holder = if grouped {
                doc.group_layers(&[id], "親").unwrap()
            } else {
                id
            };
            doc.set_layer_locks(holder, lock).unwrap();
            doc.clear_history().unwrap();
            let revision = doc.revision();
            assert_eq!(
                doc.transform_layer(id, Affine2D::translation(2., 1.), Resampling::Nearest, true),
                Err(path_error.clone()),
                "transform_layer / {lock:?} / grouped={grouped}"
            );
            // グループごと動かすときも、中のパスの層を断る
            assert_eq!(
                doc.transform_layers(
                    &[holder],
                    Affine2D::translation(2., 1.),
                    Resampling::Nearest
                ),
                Err(path_error.clone()),
                "transform_layers / {lock:?} / grouped={grouped}"
            );
            assert_eq!(doc.revision(), revision);
            assert_eq!(doc.undo_count(), 0);
        }
    }
}

/// パスを外す（rasterize）と、普通に塗れる（ロックの検査だけを通る）。
#[test]
fn a_rasterized_path_layer_is_painted_by_hand_again() {
    let mut r = rig();
    let id = r.path_layer;
    assert!(r
        .doc
        .fill(
            id,
            Channel::Color,
            Rgba8::new(9, 9, 9, 255),
            1.,
            None,
            false
        )
        .is_err());
    r.doc.rasterize(id).unwrap();
    r.doc
        .fill(
            id,
            Channel::Color,
            Rgba8::new(9, 9, 9, 255),
            1.,
            None,
            false,
        )
        .unwrap();
}
