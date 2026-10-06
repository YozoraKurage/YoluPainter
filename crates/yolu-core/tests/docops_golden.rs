//! 実 C# Core の正解との比較。台本・人工画素・状態の並びは DocOpsGolden.cs と対。
mod golden_update;
use std::path::Path;
use yolu_core::*;
fn pattern(x: u32, y: u32, c: u32, seed: u32) -> Rgba8 {
    let n = x * 17 + y * 31 + c * 13 + seed * 7;
    Rgba8::new(
        (n * 3 + 1) as u8,
        (n * 5 + 2) as u8,
        (n * 7 + 3) as u8,
        if n.is_multiple_of(5) {
            0
        } else if n % 5 == 1 {
            255
        } else {
            (n % 256) as u8
        },
    )
}
fn make(seed: u32) -> Document {
    let mut d = Document::with_tile_size(17, 13, [4, 8, 16][seed as usize % 3]).unwrap();
    for l in 0..2 {
        let a = d.add_layer(&format!("layer{l}")).unwrap();
        for c in Channel::ALL {
            d.set_channel_enabled(a, c, true).unwrap();
            for y in 0..13 {
                for x in 0..17 {
                    if !(x + y + seed + l).is_multiple_of(4) {
                        d.set_channel_pixel(a, c, x, y, pattern(x, y, c.index() as u32, seed + l))
                            .unwrap();
                    }
                }
            }
        }
        d.add_layer_mask(a).unwrap();
        for y in 0..13 {
            for x in 0..17 {
                if (x + y) % 5 == 0 {
                    d.set_mask_pixel(a, x, y, (x * 11 + y * 7) as u8).unwrap();
                }
            }
        }
    }
    d.clear_history().unwrap();
    d
}
fn i32b(out: &mut Vec<u8>, n: i32) {
    out.extend_from_slice(&n.to_le_bytes());
}
fn f64b(out: &mut Vec<u8>, n: f64) {
    out.extend_from_slice(&n.to_le_bytes());
}
fn u64b(out: &mut Vec<u8>, n: u64) {
    out.extend_from_slice(&n.to_le_bytes());
}
fn snapshot(out: &mut Vec<u8>, d: &Document) {
    i32b(out, d.width() as i32);
    i32b(out, d.height() as i32);
    i32b(out, d.layers().len() as i32);
    f64b(out, d.normal_settings().strength());
    out.push(d.selection().is_some() as u8);
    if let Some(m) = d.selection() {
        out.extend(m.to_canvas_bytes());
    }
    for l in d.layers() {
        i32b(out, l.kind() as i32);
        i32b(
            out,
            l.parent().map_or(-1, |p| d.layer_index(p).unwrap() as i32),
        );
        out.push(l.visible() as u8);
        f64b(out, l.opacity());
        i32b(out, l.blend_mode() as i32);
        out.push(l.clipping() as u8);
        i32b(out, l.locks().bits() as i32);
        for c in Channel::ALL {
            out.push(l.is_channel_enabled(c) as u8);
            out.push(l.surface(c).is_some() as u8);
            i32b(out, l.blend_mode_in(c) as i32);
            f64b(out, l.opacity_in(c));
            for y in 0..d.height() {
                for x in 0..d.width() {
                    out.extend_from_slice(&l.pixel(c, x, y).unwrap().to_array());
                }
            }
        }
        out.push(l.mask().is_some() as u8);
        if let Some(m) = l.mask() {
            out.push(m.enabled() as u8);
            out.push(m.inverted() as u8);
            f64b(out, m.density());
            for y in 0..d.height() {
                for x in 0..d.width() {
                    out.push(m.surface().pixel(x, y).unwrap().a);
                }
            }
        }
    }
    for c in Channel::ALL {
        out.extend(d.composite_channel(c, d.bounds()).unwrap());
    }
}
fn transform(n: u32) -> Affine2D {
    match n {
        0 => Affine2D::translation(3., -2.),
        1 => Affine2D::translation(0.25, -0.75),
        2 => Affine2D::from_parts((8.5, 6.5), (0., 0.), 90., (1., 1.)).unwrap(),
        3 => Affine2D::from_parts((8.5, 6.5), (0., 0.), 180., (1., 1.)).unwrap(),
        4 => Affine2D::from_parts((8.5, 6.5), (0., 0.), 0., (-1., 1.)).unwrap(),
        5 => Affine2D::from_parts((8.5, 6.5), (1., -1.), 23., (1.3, 0.7)).unwrap(),
        6 => Affine2D::from_parts((0., 0.), (0., 0.), 0., (2., 2.)).unwrap(),
        _ => Affine2D::from_parts((8., 6.), (0., 0.), -31., (0.5, 1.7)).unwrap(),
    }
}
fn report(out: &mut Vec<u8>, r: LayerMergeReport) {
    i32b(out, r.method as i32);
    i32b(out, r.notes as i32);
    u64b(out, r.compared_pixels);
    u64b(out, r.changed_pixels);
    i32b(out, r.max_difference as i32);
    i32b(out, r.max_visible_difference as i32);
    for c in Channel::ALL {
        u64b(out, r.changed_by_channel.get(&c).copied().unwrap_or(0));
    }
}
/// 中ほどだけ量のある選択範囲（量は 0 のこともある）。DocOpsGolden.cs の selected_ と同じ。
fn partial_selection(d: &Document) -> SelectionMask {
    let ts = d.tile_size();
    let mut tiles = Vec::new();
    for coord in d.canvas_tiles() {
        let mut v = vec![0; (ts * ts) as usize];
        for y in 0..ts.min(d.height() - coord.y * ts) {
            for x in 0..ts.min(d.width() - coord.x * ts) {
                let px = coord.x * ts + x;
                let py = coord.y * ts + y;
                if (3..14).contains(&px) && (2..11).contains(&py) {
                    v[(y * ts + x) as usize] = ((px * 17 + py * 29) % 256) as u8;
                }
            }
        }
        if v.iter().any(|&a| a != 0) {
            tiles.push((coord, v));
        }
    }
    SelectionMask::from_amount_tiles(d.width(), d.height(), ts, tiles).unwrap()
}
/// .NET の BinaryWriter.Write(string)（127 バイトまでの長さの前置き + UTF-8）。
fn string_b(out: &mut Vec<u8>, s: &str) {
    assert!(s.len() < 128);
    out.push(s.len() as u8);
    out.extend_from_slice(s.as_bytes());
}
enum Merge {
    Down(LayerId),
    Visible,
    Layers(Vec<LayerId>),
    Group(LayerId),
}
/// 結合がロックで断られる・断られない（DocOpsGolden.cs の MergeLock と同じ番号）。結果の型（0 成功・1 ロック・2 そのほかの拒否）、
/// 断った層・持ち主・ロック、断ったあとの文書を書く。
fn merge_lock(d: &mut Document, n: u32) -> Vec<u8> {
    let (a, b) = (d.layers()[0].id(), d.layers()[1].id());
    let lock = |d: &mut Document, id, locks| d.set_layer_locks(id, locks).unwrap();
    let group = |d: &mut Document, ids: &[LayerId]| d.group_layers(ids, "g").unwrap();
    let both = vec![a, b];
    let act = match n {
        0 => {
            lock(d, b, LayerLocks::PIXELS);
            Merge::Down(b)
        }
        1 => {
            lock(d, b, LayerLocks::ALL);
            Merge::Down(b)
        }
        2 => {
            lock(d, a, LayerLocks::PIXELS);
            Merge::Down(b)
        }
        3 => {
            lock(d, a, LayerLocks::ALL);
            Merge::Down(b)
        }
        4 => {
            lock(d, a, LayerLocks::TRANSPARENCY);
            Merge::Down(b)
        }
        5 => {
            lock(d, b, LayerLocks::TRANSPARENCY);
            Merge::Down(b)
        }
        6 => {
            lock(d, a, LayerLocks::POSITION);
            Merge::Down(b)
        }
        7 => {
            d.set_layer_clipping(b, true).unwrap();
            lock(d, a, LayerLocks::TRANSPARENCY);
            Merge::Down(b)
        }
        8..=10 => {
            let g = group(d, &both);
            lock(
                d,
                g,
                [
                    LayerLocks::ALL,
                    LayerLocks::PIXELS,
                    LayerLocks::TRANSPARENCY,
                ][n as usize - 8],
            );
            Merge::Down(b)
        }
        11 => {
            lock(d, b, LayerLocks::PIXELS);
            Merge::Visible
        }
        12 => {
            lock(d, b, LayerLocks::PIXELS);
            d.set_layer_visible(b, false).unwrap();
            Merge::Visible
        }
        13 => {
            let g = group(d, &both);
            lock(d, g, LayerLocks::ALL);
            Merge::Visible
        }
        14 => {
            lock(d, b, LayerLocks::TRANSPARENCY);
            Merge::Visible
        }
        15 => {
            lock(d, a, LayerLocks::PIXELS);
            Merge::Layers(both)
        }
        16 => {
            lock(d, b, LayerLocks::ALL);
            Merge::Layers(both)
        }
        17 => {
            let g = group(d, &both);
            lock(d, g, LayerLocks::ALL);
            Merge::Layers(both)
        }
        18 => {
            lock(d, a, LayerLocks::TRANSPARENCY);
            Merge::Layers(both)
        }
        19 => {
            let g = group(d, &both);
            lock(d, g, LayerLocks::PIXELS);
            Merge::Group(g)
        }
        20 => {
            let g = group(d, &both);
            lock(d, a, LayerLocks::ALL);
            Merge::Group(g)
        }
        21 => {
            let g = group(d, &both);
            lock(d, g, LayerLocks::TRANSPARENCY);
            Merge::Group(g)
        }
        22 => {
            let g = group(d, &both);
            lock(d, g, LayerLocks::POSITION);
            Merge::Group(g)
        }
        23 => Merge::Group(a),
        24 => Merge::Group(d.add_group("empty", None).unwrap()),
        25 => {
            group(d, &[b]);
            Merge::Layers(both)
        }
        26 => {
            d.set_layer_visible(b, false).unwrap();
            Merge::Layers(both)
        }
        27 => Merge::Down(group(d, &[a])),
        28 => {
            group(d, &[a]);
            Merge::Down(b)
        }
        29 => {
            d.set_layer_visible(b, false).unwrap();
            Merge::Down(b)
        }
        30 => {
            d.set_layer_visible(a, false).unwrap();
            d.set_layer_visible(b, false).unwrap();
            Merge::Visible
        }
        31 => Merge::Down(a),
        32 => {
            d.set_layer_clipping(b, true).unwrap();
            lock(d, b, LayerLocks::PIXELS);
            Merge::Down(b)
        }
        _ => {
            let g = group(d, &both);
            d.set_layer_clipping(b, true).unwrap();
            lock(d, g, LayerLocks::TRANSPARENCY);
            Merge::Down(b)
        }
    };
    d.clear_history().unwrap();
    let result = match act {
        Merge::Down(id) => d.merge_down(id, 255),
        Merge::Visible => d.merge_visible("merged", 255),
        Merge::Layers(ids) => d.merge_layers(&ids, 255),
        Merge::Group(id) => d.merge_group(id, 255),
    };
    let mut out = Vec::new();
    let merged = match result {
        Ok(r) => {
            out.push(0);
            report(&mut out, r);
            true
        }
        Err(CoreError::LayerLocked {
            layer,
            holder,
            lock,
        }) => {
            out.push(1);
            i32b(&mut out, d.layer_index(layer).unwrap() as i32);
            i32b(&mut out, d.layer_index(holder).unwrap() as i32);
            i32b(&mut out, lock.bits() as i32);
            false
        }
        Err(CoreError::MergeRefused(r)) => {
            out.push(2);
            string_b(&mut out, &format!("{r:?}"));
            false
        }
        Err(e) => panic!("結合 {n}: 想定外の失敗 {e:?}"),
    };
    u64b(&mut out, d.history_bytes());
    snapshot(&mut out, d);
    if merged {
        d.undo().unwrap();
        u64b(&mut out, d.history_bytes());
        snapshot(&mut out, d);
        d.redo().unwrap();
        u64b(&mut out, d.history_bytes());
        snapshot(&mut out, d);
    }
    out
}
fn run(op: &str, n: u32, seed: u32) -> Vec<u8> {
    let mut d = make(seed);
    let a = d.layers()[0].id();
    let b = d.layers()[1].id();
    let mut out = Vec::new();
    let mut r = None;
    let op = if let Some(op) = op.strip_prefix("selected_") {
        d.set_selection(Some(partial_selection(&d))).unwrap();
        d.clear_history().unwrap();
        op
    } else {
        op
    };
    match op {
        "region" => {
            let region = SelectionMask::rectangle(&d, 5, 1, 16, 8);
            d.transform_layer_region(a, transform(n % 8), Resampling::Bilinear, true, &region)
                .unwrap();
        }
        "fill" => {
            if n < 4 {
                d.set_layer_locks(a, LayerLocks::TRANSPARENCY).unwrap();
            }
            d.clear_history().unwrap();
            let region = (!n.is_multiple_of(2)).then(|| SelectionMask::rectangle(&d, 2, 1, 10, 9));
            d.fill(
                a,
                Channel::Color,
                Rgba8::new(70, 210, 120, 123),
                0.65,
                region.as_ref(),
                n >= 6,
            )
            .unwrap();
        }
        "transform" => {
            d.transform_layer(
                a,
                transform(n % 8),
                if n < 8 {
                    Resampling::Bilinear
                } else {
                    Resampling::Nearest
                },
                !n.is_multiple_of(3),
            )
            .unwrap();
        }
        "resize" => {
            let (w, h) = [(34, 26), (8, 6), (5, 3), (19, 11), (1, 1), (17, 26)][n as usize % 6];
            d.resize_image(
                w,
                h,
                [
                    CanvasResampling::Nearest,
                    CanvasResampling::Bilinear,
                    CanvasResampling::Area,
                ][n as usize / 6],
            )
            .unwrap();
            snapshot(&mut out, &d);
            return out;
        }
        "mergelock" => return merge_lock(&mut d, n),
        "bounds" => {
            let mut target = if n.is_multiple_of(2) { a } else { b };
            let mut region = None;
            if n == 2 || n == 3 {
                region = Some(SelectionMask::rectangle(&d, 5, 1, 16, 8));
            }
            if n == 4 {
                target = d.add_layer("empty").unwrap();
            }
            if n == 5 {
                target = d.add_layer("maskonly").unwrap();
                d.add_layer_mask(target).unwrap();
                d.set_mask_pixel(target, 3, 4, 200).unwrap();
                d.set_mask_pixel(target, 9, 10, 7).unwrap();
            }
            if n == 6 {
                target = d.add_layer("clear").unwrap();
                d.set_pixel(target, 5, 5, Rgba8::new(10, 20, 30, 0))
                    .unwrap();
            }
            if n == 7 {
                region = Some(partial_selection(&d));
            }
            d.clear_history().unwrap();
            let bounds = d.transform_bounds(target, region.as_ref()).unwrap();
            out.push(bounds.is_some() as u8);
            if let Some(r) = bounds {
                i32b(&mut out, r.x as i32);
                i32b(&mut out, r.y as i32);
                i32b(&mut out, (r.x + r.width) as i32);
                i32b(&mut out, (r.y + r.height) as i32);
            }
            u64b(&mut out, d.history_bytes());
            snapshot(&mut out, &d);
            return out;
        }
        "effect" => {
            d.set_layer_locks(a, LayerLocks::TRANSPARENCY).unwrap();
            d.clear_history().unwrap();
            let base = BrushSettings {
                radius: 4.,
                hardness: 0.6,
                spacing: 0.2,
                color: Rgba8::new(200, 90, 30, 255),
                opacity: 0.7,
                flow: 0.6,
                pressure_size: false,
                pressure_opacity: false,
                ..Default::default()
            };
            let mut brush = Brush::from(base);
            brush.effect = match n {
                0 => BrushEffect::Blur { radius: 2 },
                1 => BrushEffect::Smudge { strength: 0.6 },
                _ => BrushEffect::Clone {
                    offset: glam::DVec2::new(-2.25, 0.5),
                },
            };
            let mut stroke = d.begin_brush_stroke(a, &brush).unwrap();
            for (x, y) in [(5., 5.), (7., 6.), (11., 7.)] {
                stroke
                    .add_point(&mut d, x, y, 1., glam::DVec2::ZERO)
                    .unwrap();
            }
            d.end_stroke(stroke).unwrap();
        }
        "lock" => {
            d.set_layer_locks(a, LayerLocks::TRANSPARENCY).unwrap();
            d.clear_history().unwrap();
            let brush = BrushSettings {
                radius: 80.,
                hardness: if n == 0 { 1. } else { 0.4 },
                color: Rgba8::new(220, 30, 80, 128),
                opacity: 0.7,
                flow: 0.8,
                pressure_size: false,
                pressure_opacity: false,
                ..Default::default()
            };
            let mut s = d.begin_stroke(a, &brush).unwrap();
            s.add_point(&mut d, 8., 6., 1., glam::DVec2::ZERO).unwrap();
            s.add_point(&mut d, 9., 6., 1., glam::DVec2::ZERO).unwrap();
            d.end_stroke(s).unwrap();
        }
        "multi" => {
            let c = d.add_layer("empty").unwrap();
            let g = d.group_layers(&[a], "group").unwrap();
            d.clear_history().unwrap();
            match n {
                0 => {
                    d.duplicate_layers(&[a, g, c]).unwrap();
                }
                1 => {
                    d.remove_layers(&[a, g, c]).unwrap();
                }
                2 => {
                    d.set_layers_visibility(&[a, g, b], false).unwrap();
                }
                3 => {
                    d.move_layers(&[b, c], Some(g), 0).unwrap();
                }
                4 => {
                    d.step_layers(&[g, b], true).unwrap();
                }
                5 => {
                    d.change_layer_locks(
                        &[g, b, b],
                        LayerLocks::POSITION | LayerLocks::PIXELS,
                        true,
                    )
                    .unwrap();
                }
                _ => {
                    d.transform_layers(&[g, b, a], transform(5), Resampling::Bilinear)
                        .unwrap();
                }
            }
        }
        _ => {
            match n % 8 {
                0 => d.set_layer_clipping(b, true).unwrap(),
                1 => {
                    d.set_layer_opacity(a, 0.4, false).unwrap();
                    d.set_layer_blend_mode(a, BlendMode::Multiply).unwrap();
                }
                2 => {
                    let bg = d
                        .add_fill_layer(
                            "background",
                            &[(Channel::Color, Rgba8::new(100, 200, 70, 255))],
                            None,
                        )
                        .unwrap();
                    d.move_layer(bg, 0).unwrap();
                }
                3 => d.set_layer_blend_mode(b, BlendMode::Screen).unwrap(),
                4 => d.set_channel_enabled(a, Channel::Normal, false).unwrap(),
                5 => d.set_channel_enabled(b, Channel::Metallic, false).unwrap(),
                6 => {
                    d.set_layer_mask_inverted(a, true).unwrap();
                    d.set_layer_mask_density(a, 0.35, false).unwrap();
                }
                _ => {
                    d.set_channel_blend(
                        a,
                        Channel::Color,
                        ChannelBlend::new(Some(BlendMode::Multiply), Some(0.65)),
                        false,
                    )
                    .unwrap();
                    d.set_layer_opacity(b, 0.55, false).unwrap();
                }
            }
            match op {
                "merge_down" => {
                    d.clear_history().unwrap();
                    r = Some(d.merge_down(b, 255).unwrap());
                }
                "merge_visible" => {
                    let h = d.duplicate_layer(a, None).unwrap();
                    d.set_layer_visible(h, false).unwrap();
                    d.group_layers(&[a, h], "group").unwrap();
                    d.clear_history().unwrap();
                    r = Some(d.merge_visible("merged", 255).unwrap());
                }
                "merge_group" => {
                    let g = d.group_layers(&[a, b], "group").unwrap();
                    d.set_layer_blend_mode(g, BlendMode::Multiply).unwrap();
                    d.clear_history().unwrap();
                    r = Some(d.merge_group(g, 255).unwrap());
                }
                _ => {
                    d.clear_history().unwrap();
                    r = Some(d.merge_layers(&[a, b], 255).unwrap());
                }
            }
        }
    }
    if let Some(r) = r {
        report(&mut out, r);
    }
    u64b(&mut out, d.history_bytes());
    snapshot(&mut out, &d);
    d.undo().unwrap();
    u64b(&mut out, d.history_bytes());
    snapshot(&mut out, &d);
    d.redo().unwrap();
    u64b(&mut out, d.history_bytes());
    snapshot(&mut out, &d);
    out
}
#[test]
fn csharp_docops_all_bytes_and_parallelism() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/docops");
    for degree in [1, 4] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(degree)
            .build()
            .unwrap();
        pool.install(|| {
            for line in include_str!("golden/docops/cases.txt")
                .lines()
                .filter(|l| !l.starts_with('#') && !l.is_empty())
            {
                let p: Vec<_> = line.split_whitespace().collect();
                let expected = std::fs::read(root.join(format!("{}.bin", p[0]))).unwrap();
                let actual = run(p[1], p[2].parse().unwrap(), p[3].parse().unwrap());
                if actual != expected && golden_update::updating() {
                    if degree == 1 {
                        std::fs::write(root.join(format!("{}.bin", p[0])), &actual).unwrap();
                    }
                    continue;
                }
                assert_eq!(
                    actual.len(),
                    expected.len(),
                    "{} degree={degree} 長さ",
                    p[0]
                );
                if let Some(i) = actual.iter().zip(&expected).position(|(a, b)| a != b) {
                    panic!(
                        "{} degree={degree} offset={i} Rust={} C#={}（{} bytes）",
                        p[0],
                        actual[i],
                        expected[i],
                        actual.len()
                    );
                }
            }
        });
    }
}
