//! 効果のキャッシュの整合: どんな編集の並びでも、キャッシュを使った合成が、キャッシュを捨てて評価し直した合成と同じバイトで、
//! 変化の記録（`changed_tiles`）が、前の合成との違いをすべて含む。
use crate::attach_support;
use attach_support::*;
use std::collections::HashSet;
use yolu_core::generator::{self, Settings};
use yolu_core::{
    AnchorId, AnchorPlacement, BlendMode, Channel, Document, EffectSettings, FilterId, FilterSpec,
    FilterTarget, LayerId, TileCoord,
};

fn tiles_differing(doc: &Document, a: &[u8], b: &[u8]) -> HashSet<TileCoord> {
    let ts = doc.tile_size();
    let mut set = HashSet::new();
    for (i, (p, q)) in a
        .as_chunks::<4>()
        .0
        .iter()
        .zip(b.as_chunks::<4>().0)
        .enumerate()
    {
        if p != q {
            let (x, y) = (i as u32 % doc.width(), i as u32 / doc.width());
            set.insert(TileCoord::new(x / ts, y / ts));
        }
    }
    set
}

fn random_settings(rng: &mut Rng, channel: Channel) -> EffectSettings {
    match rng.below(9) {
        0 => EffectSettings::blur(1 + rng.below(12) as u32),
        1 => EffectSettings::sharpen(
            1 + rng.below(4) as u32,
            rng.unit() * 2.0,
            rng.below(20) as u32,
        ),
        2 => EffectSettings::noise(rng.unit(), rng.below(100) as i32, true),
        3 => EffectSettings::levels(0.1, 0.9, 1.0 + rng.unit(), 0.1, 0.9),
        4 => EffectSettings::invert(),
        5 => EffectSettings::normalize(),
        6 => {
            let mut g = Settings::new(generator::Kind::PositionGradient);
            g.axis = rng.below(3);
            g.blend = generator::Blend::Multiply;
            EffectSettings::generator(g)
        }
        7 => {
            let mut g = Settings::new(generator::Kind::EdgeWear);
            g.noise_amount = 0.0;
            EffectSettings::generator(g)
        }
        _ => {
            let _ = channel;
            let mut g = Settings::new(generator::Kind::Anchor);
            g.blend = generator::Blend::Replace;
            EffectSettings::generator(g)
        }
    }
}

struct Run {
    doc: Document,
    layers: Vec<LayerId>,
    rng: Rng,
    anchors: Vec<AnchorId>,
}

impl Run {
    fn step(&mut self) {
        let doc = &mut self.doc;
        let rng = &mut self.rng;
        let layer = self.layers[rng.below(self.layers.len())];
        let channel = [Channel::Color, Channel::Height][rng.below(2)];
        match rng.below(26) {
            22 | 23 | 24 | 0 | 1 => {
                // 描く（元画素の変化）
                if let Some(l) = doc.layer(layer) {
                    if l.kind() == yolu_core::LayerKind::Raster && l.surface(channel).is_some() {
                        stroke(doc, layer, channel, rng);
                    }
                }
            }
            2..=4 => {
                let _ = doc.add_filter(
                    layer,
                    FilterTarget::Content,
                    FilterSpec::new(random_settings(rng, channel))
                        .channels(&[channel])
                        .strength(if rng.below(4) == 0 { 0.5 } else { 1.0 }),
                );
            }
            5 => {
                let stack: Vec<FilterId> = doc
                    .layer(layer)
                    .map(|l| l.filters().iter().map(|e| e.id()).collect())
                    .unwrap_or_default();
                if !stack.is_empty() {
                    let f = stack[rng.below(stack.len())];
                    match rng.below(4) {
                        0 => doc.remove_filter(layer, f).unwrap(),
                        1 => doc.move_filter(layer, f, 0).unwrap(),
                        2 => doc.set_filter_enabled(layer, f, rng.below(2) == 0).unwrap(),
                        _ => {
                            let _ = doc.set_filter_strength(layer, f, rng.unit(), false);
                        }
                    }
                }
            }
            6 => {
                // マスクとそのフィルター
                if doc.layer(layer).is_some_and(|l| l.mask().is_none()) {
                    doc.add_layer_mask(layer).unwrap();
                }
                let _ = doc.add_filter(
                    layer,
                    FilterTarget::Mask,
                    FilterSpec::new(random_settings(rng, channel)),
                );
            }
            7 => {
                if doc.layer(layer).is_some_and(|l| l.mask().is_some()) {
                    let brush = yolu_core::BrushSettings::default();
                    let mut s = doc.begin_mask_stroke(layer, &brush).unwrap();
                    s.add_point(
                        doc,
                        5.0 + rng.unit() * 20.0,
                        5.0 + rng.unit() * 10.0,
                        1.0,
                        yolu_core::glam::DVec2::ZERO,
                    )
                    .unwrap();
                    doc.end_stroke(s).unwrap();
                }
            }
            8 => {
                // Anchor を置く・外す・名前
                match rng.below(3) {
                    0 => {
                        let placement = if rng.below(3) == 0 {
                            AnchorPlacement::Mask
                        } else {
                            AnchorPlacement::Layer
                        };
                        if let Ok(id) = doc.add_anchor(layer, placement, None, None) {
                            self.anchors.push(id);
                        }
                    }
                    1 => {
                        if let Some(a) = self.anchors.pop() {
                            let _ = doc.remove_anchor(a);
                        }
                    }
                    _ => {
                        if let Some(a) = self.anchors.last() {
                            let _ = doc.rename_anchor(*a, "別の名前");
                        }
                    }
                }
            }
            9 | 10 => {
                // Anchor の Generator が読む Anchor を選ぶ
                let stages: Vec<FilterId> = doc
                    .layer(layer)
                    .map(|l| {
                        l.filters()
                            .iter()
                            .filter(|e| e.settings().reads_anchor())
                            .map(|e| e.id())
                            .collect()
                    })
                    .unwrap_or_default();
                if let (Some(f), false) = (stages.first(), self.anchors.is_empty()) {
                    let a = self.anchors[rng.below(self.anchors.len())];
                    let _ = doc.set_generator_anchor(
                        layer,
                        *f,
                        Some(a),
                        channel,
                        if rng.below(2) == 0 {
                            generator::anchor::ReadMode::Value
                        } else {
                            generator::anchor::ReadMode::Coverage
                        },
                        false,
                    );
                }
            }
            11 => {
                let _ = doc.set_layer_opacity(layer, rng.unit(), false);
            }
            12 => {
                let _ = doc.set_layer_visible(layer, rng.below(4) != 0);
            }
            13 => {
                let _ = doc.set_layer_blend_mode(
                    layer,
                    [BlendMode::Normal, BlendMode::Multiply, BlendMode::Overlay][rng.below(3)],
                );
            }
            14 => {
                let _ = doc.move_layer(layer, rng.below(4));
            }
            15 => {
                let _ = doc.set_layer_clipping(layer, rng.below(2) == 0);
            }
            16 => {
                let _ = doc.undo();
            }
            17 => {
                let _ = doc.redo();
            }
            18 => {
                // 入力を替える・同じものを渡し直す
                let seed = if rng.below(3) == 0 { 7 } else { 0 };
                doc.set_effect_inputs(inputs(seed)).unwrap();
            }
            19 => {
                // 塗りつぶしの画像・グラデーション・投影
                let fill = self.layers[3];
                match rng.below(3) {
                    0 => {
                        let _ = doc.set_fill_image(
                            fill,
                            Channel::Color,
                            if rng.below(3) == 0 {
                                None
                            } else {
                                Some(image(rng.below(2)))
                            },
                        );
                    }
                    1 => {
                        let mut g = Settings::new(generator::Kind::ShapeGradient);
                        g.ramp = Some(generator::Ramp::default());
                        g.blend = generator::Blend::Replace;
                        g.volume.falloff = rng.unit();
                        let _ = doc.set_fill_gradient(fill, Channel::Height, Some(g), false);
                    }
                    _ => {
                        let p = yolu_core::fill_image::Projection {
                            tiles: [1.0 + rng.below(3) as f64, 1.0],
                            ..Default::default()
                        };
                        let _ = doc.set_fill_projection(fill, p, false);
                    }
                }
            }
            20 => {
                let _ = doc.set_layer_mask_inverted(layer, rng.below(2) == 0);
            }
            _ => {
                let _ = doc.set_layer_mask_density(layer, rng.unit(), false);
            }
        }
    }
}

#[test]
fn the_cache_never_shows_stale_output_and_changed_tiles_cover_every_difference() {
    for seed in 0..24u64 {
        let (doc, layers) = world();
        let mut run = Run {
            doc,
            layers,
            rng: Rng(seed * 7919 + 1),
            anchors: Vec::new(),
        };
        let mut previous: Vec<Vec<u8>> = [Channel::Color, Channel::Height]
            .iter()
            .map(|c| whole(&run.doc, *c))
            .collect();
        let mut since = run.doc.change_serial();
        for step in 0..120 {
            run.step();
            let now: Vec<Vec<u8>> = [Channel::Color, Channel::Height]
                .iter()
                .map(|c| whole(&run.doc, *c))
                .collect();
            run.doc.release_effect_cache();
            let fresh: Vec<Vec<u8>> = [Channel::Color, Channel::Height]
                .iter()
                .map(|c| whole(&run.doc, *c))
                .collect();
            assert!(
                now == fresh,
                "seed {seed} step {step}: キャッシュを使った合成が、評価し直した合成と違う"
            );
            for (i, c) in [Channel::Color, Channel::Height].iter().enumerate() {
                let changed: HashSet<TileCoord> = run
                    .doc
                    .changed_tiles(*c, since)
                    .unwrap()
                    .into_iter()
                    .collect();
                let differing = tiles_differing(&run.doc, &previous[i], &now[i]);
                let missing: Vec<_> = differing.difference(&changed).collect();
                assert!(
                    missing.is_empty(),
                    "seed {seed} step {step} {c:?}: 変わったタイルが変化の記録に無い: {missing:?}"
                );
            }
            previous = now;
            since = run.doc.change_serial();
        }
        let filters: usize = run.doc.layers().iter().map(|l| l.filters().len()).sum();
        let readers = run
            .doc
            .layers()
            .iter()
            .flat_map(|l| l.filters())
            .filter(|e| e.settings().reads_anchor())
            .count();
        eprintln!(
            "seed {seed}: 段 {filters}（Anchor を読む {readers}）、Anchor {}、評価したブロック {}",
            run.doc.anchors().len(),
            run.doc.effect_counters().blocks_evaluated
        );
    }
}

/// Anchor を読む Generator の段を足して、読む Anchor を選ぶ。
fn anchor_stage(
    doc: &mut Document,
    layer: LayerId,
    target: FilterTarget,
    channels: &[Channel],
    anchor: AnchorId,
    read_channel: Channel,
    blend: generator::Blend,
) -> FilterId {
    let mut g = Settings::new(generator::Kind::Anchor);
    g.blend = blend;
    let spec = FilterSpec::new(EffectSettings::generator(g));
    let spec = if channels.is_empty() {
        spec
    } else {
        spec.channels(channels)
    };
    let id = doc.add_filter(layer, target, spec).unwrap();
    doc.set_generator_anchor(
        layer,
        id,
        Some(anchor),
        read_channel,
        generator::anchor::ReadMode::Value,
        false,
    )
    .unwrap();
    id
}

/// Anchor を読む段が決まっている文書で、元画素・マスク・層の属性・並びを動かす（Anchor の値の変化が、読む層の出力と変化の記録へ届く）。
fn anchor_world() -> (Document, Vec<LayerId>) {
    let (mut doc, layers) = world();
    let (base, mid, top, fill) = (layers[0], layers[1], layers[2], layers[3]);
    doc.add_layer_mask(base).unwrap();
    let a_base = doc
        .add_anchor(base, AnchorPlacement::Layer, None, None)
        .unwrap();
    let a_mask = doc
        .add_anchor(base, AnchorPlacement::Mask, None, None)
        .unwrap();
    let a_mid = doc
        .add_anchor(mid, AnchorPlacement::Layer, None, None)
        .unwrap();
    let stage = anchor_stage;
    // 色の層が、土台の Height を読む（読むチャンネルと出すチャンネルが違う）。その後にぼかし
    stage(
        &mut doc,
        top,
        FilterTarget::Content,
        &[Channel::Color],
        a_base,
        Channel::Height,
        generator::Blend::Multiply,
    );
    doc.add_filter(
        top,
        FilterTarget::Content,
        FilterSpec::new(EffectSettings::blur(5)).channels(&[Channel::Color]),
    )
    .unwrap();
    // マスクが土台のマスクの Anchor を読む
    doc.add_layer_mask(top).unwrap();
    stage(
        &mut doc,
        top,
        FilterTarget::Mask,
        &[],
        a_mask,
        Channel::Height,
        generator::Blend::Replace,
    );
    // 塗りつぶしの Height が中の Anchor を読む
    stage(
        &mut doc,
        fill,
        FilterTarget::Content,
        &[Channel::Height],
        a_mid,
        Channel::Height,
        generator::Blend::Min,
    );
    // 連鎖: 色の層の Anchor（Color）を、塗りつぶしの Color が読む
    let a_top = doc
        .add_anchor(top, AnchorPlacement::Layer, None, None)
        .unwrap();
    stage(
        &mut doc,
        fill,
        FilterTarget::Content,
        &[Channel::Color],
        a_top,
        Channel::Color,
        generator::Blend::Screen,
    );
    (doc, layers)
}

/// 1 回の編集の後に: キャッシュを使った合成が評価し直しと同じバイトで、変化の記録が前との違いを全部含む。違ったタイルの数を返す。
fn check_after_edit(
    doc: &mut Document,
    previous: &mut Vec<Vec<u8>>,
    since: &mut u64,
    context: &str,
) -> usize {
    let channels = [Channel::Color, Channel::Height];
    let now: Vec<Vec<u8>> = channels.iter().map(|c| whole(doc, *c)).collect();
    doc.release_effect_cache();
    let fresh: Vec<Vec<u8>> = channels.iter().map(|c| whole(doc, *c)).collect();
    assert!(
        now == fresh,
        "{context}: キャッシュの合成が評価し直しと違う"
    );
    let mut deltas = 0;
    for (i, c) in channels.iter().enumerate() {
        let changed: HashSet<TileCoord> =
            doc.changed_tiles(*c, *since).unwrap().into_iter().collect();
        let differing = tiles_differing(doc, &previous[i], &now[i]);
        deltas += differing.len();
        let missing: Vec<_> = differing.difference(&changed).collect();
        assert!(
            missing.is_empty(),
            "{context} {c:?}: 変わったタイルが変化の記録に無い: {missing:?}"
        );
    }
    *previous = now;
    *since = doc.change_serial();
    deltas
}

/// Anchor を読む文書で、元画素・マスク（画素・有効・反転・濃度・付け外し・フィルター）・層の属性・並びを乱数で動かす。
fn walk_anchor_world(make: fn() -> (Document, Vec<LayerId>), seeds: u64, steps: usize) -> usize {
    let mut deltas = 0usize;
    for seed in 0..seeds {
        let (mut doc, layers) = make();
        let mut rng = Rng(seed * 104729 + 3);
        let channels = [Channel::Color, Channel::Height];
        let mut previous: Vec<Vec<u8>> = channels.iter().map(|c| whole(&doc, *c)).collect();
        let mut since = doc.change_serial();
        for step in 0..steps {
            let layer = layers[rng.below(3)];
            let channel = channels[rng.below(2)];
            match rng.below(16) {
                0..=3 => {
                    if doc
                        .layer(layer)
                        .is_some_and(|l| l.surface(channel).is_some())
                    {
                        stroke(&mut doc, layer, channel, &mut rng);
                    }
                }
                4 => {
                    if doc.layer(layer).is_some_and(|l| l.mask().is_some()) {
                        let brush = yolu_core::BrushSettings::default();
                        let mut s = doc.begin_mask_stroke(layer, &brush).unwrap();
                        s.add_point(
                            &mut doc,
                            4.0 + rng.unit() * 30.0,
                            4.0 + rng.unit() * 18.0,
                            1.0,
                            yolu_core::glam::DVec2::ZERO,
                        )
                        .unwrap();
                        doc.end_stroke(s).unwrap();
                    }
                }
                5 => {
                    let _ = doc.set_layer_opacity(layer, rng.unit(), false);
                }
                6 => {
                    let _ = doc.set_layer_visible(layer, rng.below(3) != 0);
                }
                7 => {
                    let _ = doc.move_layer(layer, rng.below(4));
                }
                8 => {
                    let _ = doc.undo();
                }
                9 => {
                    let _ = doc.redo();
                }
                10 => {
                    let _ = doc.set_layer_mask_inverted(layer, rng.below(2) == 0);
                }
                11 => {
                    let _ = doc.set_layer_mask_density(layer, rng.unit(), false);
                }
                12 => {
                    let _ = doc.set_layer_mask_enabled(layer, rng.below(3) != 0);
                }
                13 => {
                    // マスクの付け外し
                    if doc.layer(layer).is_some_and(|l| l.mask().is_some()) {
                        let _ = doc.remove_layer_mask(layer);
                    } else {
                        let _ = doc.add_layer_mask(layer);
                    }
                }
                14 => {
                    // マスクのフィルターの入れ替え
                    if doc.layer(layer).is_some_and(|l| l.mask().is_some()) {
                        let stack: Vec<FilterId> = doc
                            .layer(layer)
                            .and_then(|l| l.mask())
                            .map(|m| m.filters().iter().map(|e| e.id()).collect())
                            .unwrap_or_default();
                        if stack.is_empty() || rng.below(2) == 0 {
                            let _ = doc.add_filter(
                                layer,
                                FilterTarget::Mask,
                                FilterSpec::new(if rng.below(2) == 0 {
                                    EffectSettings::blur(1 + rng.below(6) as u32)
                                } else {
                                    EffectSettings::invert()
                                }),
                            );
                        } else {
                            let _ = doc.remove_filter(layer, stack[rng.below(stack.len())]);
                        }
                    }
                }
                _ => {
                    // 色の層の段を有効・無効に
                    let stack: Vec<FilterId> = doc
                        .layer(layer)
                        .map(|l| l.filters().iter().map(|e| e.id()).collect())
                        .unwrap_or_default();
                    if !stack.is_empty() {
                        let _ = doc.set_filter_enabled(
                            layer,
                            stack[rng.below(stack.len())],
                            rng.below(2) == 0,
                        );
                    }
                }
            }
            deltas += check_after_edit(
                &mut doc,
                &mut previous,
                &mut since,
                &format!("seed {seed} step {step}"),
            );
        }
    }
    deltas
}

#[test]
fn anchor_readers_follow_what_they_read() {
    let deltas = walk_anchor_world(anchor_world, 16, 100);
    assert!(deltas > 1000, "試験が動かしていない: {deltas}");
}

/// グループの中: Anchor を置いた層と、それを読む層が同じグループにいる（読む層が host より上）。グループのマスクの Anchor も読む。
fn grouped_anchor_world() -> (Document, Vec<LayerId>) {
    let (mut doc, layers) = world();
    let (base, mid, top, fill) = (layers[0], layers[1], layers[2], layers[3]);
    let a_mid = doc
        .add_anchor(mid, AnchorPlacement::Layer, None, None)
        .unwrap();
    let group = doc.group_layers(&[mid, top], "グループ").unwrap();
    doc.add_layer_mask(group).unwrap();
    let a_group_mask = doc
        .add_anchor(group, AnchorPlacement::Mask, None, None)
        .unwrap();
    doc.add_layer_mask(base).unwrap();
    let a_base_mask = doc
        .add_anchor(base, AnchorPlacement::Mask, None, None)
        .unwrap();
    // グループの中の上の層（top）が、同じグループの下の層（mid）の Anchor を、自分が出すのと同じチャンネル（Color）で読む。ぼかしが続く
    // （読む層が host のスタックの結果に入る向きだと、host の Anchor が読む層を評価しに行って、読む層は host の Anchor へ戻る）
    anchor_stage(
        &mut doc,
        top,
        FilterTarget::Content,
        &[Channel::Color],
        a_mid,
        Channel::Color,
        generator::Blend::Multiply,
    );
    doc.add_filter(
        top,
        FilterTarget::Content,
        FilterSpec::new(EffectSettings::blur(4)).channels(&[Channel::Color]),
    )
    .unwrap();
    // top のマスクが、土台のマスクの Anchor を読む
    doc.add_layer_mask(top).unwrap();
    anchor_stage(
        &mut doc,
        top,
        FilterTarget::Mask,
        &[],
        a_base_mask,
        Channel::Height,
        generator::Blend::Replace,
    );
    // グループの外の塗りつぶしが、グループのマスクの Anchor を読む
    anchor_stage(
        &mut doc,
        fill,
        FilterTarget::Content,
        &[Channel::Height],
        a_group_mask,
        Channel::Height,
        generator::Blend::Min,
    );
    (doc, vec![base, mid, top, fill, group])
}

#[test]
fn a_layer_above_its_host_in_the_same_group_reads_the_anchor_without_recursing() {
    // 昔の評価は host の一番外の祖先までの層を先に評価したので、host より上の読み手が host の Anchor へ戻って終わらなかった
    let (mut doc, layers) = grouped_anchor_world();
    let (mid, top) = (layers[1], layers[2]);
    let mut previous: Vec<Vec<u8>> = [Channel::Color, Channel::Height]
        .iter()
        .map(|c| whole(&doc, *c))
        .collect();
    let mut since = doc.change_serial();
    let mut rng = Rng(11);
    let mut deltas = 0;
    for step in 0..6 {
        stroke(&mut doc, mid, Channel::Color, &mut rng);
        deltas += check_after_edit(
            &mut doc,
            &mut previous,
            &mut since,
            &format!("host に描く {step}"),
        );
    }
    stroke(&mut doc, top, Channel::Color, &mut rng);
    deltas += check_after_edit(&mut doc, &mut previous, &mut since, "読む層に描く");
    assert!(deltas > 0, "host の画素の変化が読む層の出力へ届いている");
}

#[test]
fn anchors_inside_groups_follow_what_they_read() {
    let deltas = walk_anchor_world(grouped_anchor_world, 16, 100);
    assert!(deltas > 1000, "試験が動かしていない: {deltas}");
}

/// マスクを持つ層が host で、その上の層がマスクの Anchor を読む。host は画素のタイルを持たない（マスクだけを塗った空の層）か、持つ。
fn mask_host(with_pixels: bool) -> (Document, LayerId, LayerId) {
    let mut doc = Document::with_tile_size(W, H, 8).unwrap();
    let host = doc.add_layer("host").unwrap();
    if with_pixels {
        paint(&mut doc, host, Channel::Color, 5);
    }
    doc.add_layer_mask(host).unwrap();
    let brush = yolu_core::BrushSettings::default();
    let mut s = doc.begin_mask_stroke(host, &brush).unwrap();
    for (x, y) in [(6.0, 6.0), (20.0, 12.0), (30.0, 20.0)] {
        s.add_point(&mut doc, x, y, 1.0, yolu_core::glam::DVec2::ZERO)
            .unwrap();
    }
    doc.end_stroke(s).unwrap();
    let a = doc
        .add_anchor(host, AnchorPlacement::Mask, None, None)
        .unwrap();
    let reader = doc.add_layer("reader").unwrap();
    paint(&mut doc, reader, Channel::Color, 9);
    anchor_stage(
        &mut doc,
        reader,
        FilterTarget::Content,
        &[Channel::Color],
        a,
        Channel::Color,
        generator::Blend::Multiply,
    );
    doc.set_filter_block_pixels(16).unwrap();
    (doc, host, reader)
}

/// ホストのマスクの有効・反転・濃度・フィルター・付け外しは、ホストに画素のタイルが無くても、マスクの Anchor を読む層の出力
/// （評価のキャッシュ）と変化の記録へ届く。
#[test]
fn a_mask_anchor_reader_follows_every_change_of_the_hosts_mask() {
    for with_pixels in [false, true] {
        let (mut doc, host, reader) = mask_host(with_pixels);
        let mut previous: Vec<Vec<u8>> = [Channel::Color, Channel::Height]
            .iter()
            .map(|c| whole(&doc, *c))
            .collect();
        let mut since = doc.change_serial();
        let mut run = |doc: &mut Document, what: &str, edit: &dyn Fn(&mut Document)| {
            edit(doc);
            let context = format!("host の画素 {with_pixels}: {what}");
            let deltas = check_after_edit(doc, &mut previous, &mut since, &context);
            assert!(deltas > 0, "{context}: 読む層の出力が変わるはず");
        };
        run(&mut doc, "濃度", &|d| {
            d.set_layer_mask_density(host, 0.3, false).unwrap()
        });
        run(&mut doc, "反転", &|d| {
            d.set_layer_mask_inverted(host, true).unwrap()
        });
        run(&mut doc, "無効", &|d| {
            d.set_layer_mask_enabled(host, false).unwrap()
        });
        run(&mut doc, "有効に戻す", &|d| {
            d.set_layer_mask_enabled(host, true).unwrap()
        });
        run(&mut doc, "マスクのぼかし", &|d| {
            d.add_filter(
                host,
                FilterTarget::Mask,
                FilterSpec::new(EffectSettings::blur(5)),
            )
            .unwrap();
        });
        run(&mut doc, "ぼかしの Undo", &|d| {
            assert!(d.undo().unwrap());
        });
        run(&mut doc, "有効・反転・濃度の Undo", &|d| {
            for _ in 0..3 {
                assert!(d.undo().unwrap());
            }
        });
        let _ = reader;
    }
}

/// マスクのスタックが Anchor を読む host: 読む元の変化が、host のマスクを経て、host のマスクの Anchor を読む層の変化の記録へ届く。
#[test]
fn a_change_reaches_a_mask_anchor_reader_through_the_hosts_mask_filters() {
    let mut doc = Document::with_tile_size(W, H, 8).unwrap();
    let source = doc.add_layer("読まれる").unwrap();
    paint(&mut doc, source, Channel::Color, 3);
    let a_source = doc
        .add_anchor(source, AnchorPlacement::Layer, None, None)
        .unwrap();
    let host = doc.add_layer("host").unwrap();
    doc.add_layer_mask(host).unwrap();
    anchor_stage(
        &mut doc,
        host,
        FilterTarget::Mask,
        &[],
        a_source,
        Channel::Color,
        generator::Blend::Replace,
    );
    let a_mask = doc
        .add_anchor(host, AnchorPlacement::Mask, None, None)
        .unwrap();
    // 読む層は Height を出す（読まれる層の Color の変化の記録には入らない）
    let reader = doc.add_layer("reader").unwrap();
    paint(&mut doc, reader, Channel::Height, 9);
    anchor_stage(
        &mut doc,
        reader,
        FilterTarget::Content,
        &[Channel::Height],
        a_mask,
        Channel::Color,
        generator::Blend::Multiply,
    );
    doc.set_filter_block_pixels(16).unwrap();
    let mut previous: Vec<Vec<u8>> = [Channel::Color, Channel::Height]
        .iter()
        .map(|c| whole(&doc, *c))
        .collect();
    let mut since = doc.change_serial();
    let mut rng = Rng(5);
    let mut deltas = 0;
    for step in 0..8 {
        stroke(&mut doc, source, Channel::Color, &mut rng);
        deltas += check_after_edit(
            &mut doc,
            &mut previous,
            &mut since,
            &format!("読まれる層に描く {step}"),
        );
    }
    assert!(deltas > 0);
}

/// Anchor を読む段の後ろに全域の段（正規化）がある: 読む Anchor の 1 タイルの変化が、読む層の全タイルの出力を変える。
#[test]
fn a_global_stage_after_an_anchor_stage_changes_every_tile_of_the_reader() {
    let (mut doc, l) = world();
    let (base, top) = (l[0], l[2]);
    let a = doc
        .add_anchor(base, AnchorPlacement::Layer, None, None)
        .unwrap();
    anchor_stage(
        &mut doc,
        top,
        FilterTarget::Content,
        &[Channel::Color],
        a,
        Channel::Height,
        generator::Blend::Multiply,
    );
    doc.add_filter(
        top,
        FilterTarget::Content,
        FilterSpec::new(EffectSettings::normalize()).channels(&[Channel::Color]),
    )
    .unwrap();
    let mut previous: Vec<Vec<u8>> = [Channel::Color, Channel::Height]
        .iter()
        .map(|c| whole(&doc, *c))
        .collect();
    let mut since = doc.change_serial();
    let mut rng = Rng(21);
    let mut widest = 0;
    for step in 0..8 {
        stroke(&mut doc, base, Channel::Height, &mut rng);
        let now: Vec<Vec<u8>> = [Channel::Color, Channel::Height]
            .iter()
            .map(|c| whole(&doc, *c))
            .collect();
        widest = widest.max(tiles_differing(&doc, &previous[0], &now[0]).len());
        check_after_edit(
            &mut doc,
            &mut previous,
            &mut since,
            &format!("読まれる層に描く {step}"),
        );
    }
    assert!(
        widest > 4,
        "正規化が離れたタイルの出力も変えている: {widest}"
    );
}
