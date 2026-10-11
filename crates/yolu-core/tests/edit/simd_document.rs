//! タイルの合成が、画素ごとの参照（`composite_pixel`）と同じバイトになること。全合成モード・マスク・クリッピング・
//! グループ（通過・独立）・調整レイヤー（全種類）・Normal チャンネル（Normal・Overlay）を 1 つの文書に重ねて、全チャンネル・領域の端をまたぐ矩形で比べる。
//! 道の選びは環境変数 `YOLU_SIMD`（`scalar`・`sse41`・`avx2`・`neon`。その CPU にある道）で変えて同じ試験を回せる。
//!
//! 合成の本体（`composite.rs`）は行の核（`blend_row`・`clip_row`・`fade_row`・`composite_row`・Normal チャンネルの `normal::*_row`）で重ねるので、
//! この試験は SIMD の道を通る合成の回帰の網になる（参照の `composite_pixel` は画素ごとの式）。`YOLU_SIMD` で道ごとに回す。
#![allow(clippy::chunks_exact_to_as_chunks)]

use yolu_core::curve::{Curve, CurvePoint};
use yolu_core::generator::Ramp;
use yolu_core::{
    AdjustmentSettings, BlendMode, BrightnessContrast, Channel, ColorBalance, Document,
    GradientMap, LayerId, Posterize, Rect, Rgba8, Threshold, TileCoord, ToneChannel, ToneCurves,
};

struct SplitMix(u64);
impl SplitMix {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    }
    fn channel(&mut self) -> u8 {
        let r = self.next();
        match r & 7 {
            0 => 0,
            1 => 255,
            _ => (r >> 8) as u8,
        }
    }
    fn alpha(&mut self) -> u8 {
        let r = self.next();
        match r & 7 {
            0 | 1 => 0,
            2 | 3 => 255,
            _ => (r >> 8) as u8,
        }
    }
}

/// 1 タイルぶんの乱数。キャンバスの外の余白は 0（読み込みの決まり）。
fn tile_bytes(doc: &Document, tx: u32, ty: u32, rng: &mut SplitMix, mask: bool, bytes: &mut [u8]) {
    let ts = doc.tile_size();
    for (i, p) in bytes.chunks_exact_mut(4).enumerate() {
        let (x, y) = (tx * ts + i as u32 % ts, ty * ts + i as u32 / ts);
        if x >= doc.width() || y >= doc.height() {
            p.copy_from_slice(&[0, 0, 0, 0]);
        } else if mask {
            p.copy_from_slice(&[0, 0, 0, rng.alpha()]);
        } else {
            p.copy_from_slice(&[rng.channel(), rng.channel(), rng.channel(), rng.alpha()]);
        }
    }
}

fn fill(doc: &mut Document, layer: LayerId, channel: Channel, rng: &mut SplitMix) {
    let ts = doc.tile_size() as usize;
    let mut bytes = vec![0u8; ts * ts * 4];
    for ty in 0..doc.height().div_ceil(ts as u32) {
        for tx in 0..doc.width().div_ceil(ts as u32) {
            tile_bytes(doc, tx, ty, rng, false, &mut bytes);
            doc.import_tile(layer, channel, TileCoord::new(tx, ty), &bytes)
                .unwrap();
        }
    }
}

fn fill_mask(doc: &mut Document, layer: LayerId, rng: &mut SplitMix) {
    doc.add_layer_mask(layer).unwrap();
    let ts = doc.tile_size() as usize;
    let mut bytes = vec![0u8; ts * ts * 4];
    for ty in 0..doc.height().div_ceil(ts as u32) {
        for tx in 0..doc.width().div_ceil(ts as u32) {
            tile_bytes(doc, tx, ty, rng, true, &mut bytes);
            doc.import_mask_tile(layer, TileCoord::new(tx, ty), &bytes)
                .unwrap();
        }
    }
}

fn adjustments() -> Vec<AdjustmentSettings> {
    let curve = Curve::new(vec![
        CurvePoint { x: 0.0, y: 0.1 },
        CurvePoint { x: 0.4, y: 0.6 },
        CurvePoint { x: 1.0, y: 0.9 },
    ])
    .unwrap();
    vec![
        AdjustmentSettings::invert(),
        AdjustmentSettings::levels(0.1, 0.9, 1.4, 0.05, 0.95).unwrap(),
        AdjustmentSettings::hue_saturation(40.0, 0.3, 0.1).unwrap(),
        AdjustmentSettings::hue_saturation(-120.0, -0.6, -0.4).unwrap(),
        AdjustmentSettings::gradient_map(GradientMap::new(Ramp::default(), false)),
        AdjustmentSettings::tone_curve(
            ToneCurves::identity().with_curve(ToneChannel::Composite, curve),
        ),
        AdjustmentSettings::color_balance(
            ColorBalance::new(
                [10.0, -20.0, 30.0],
                [40.0, 0.0, -30.0],
                [0.0, 20.0, 50.0],
                true,
            )
            .unwrap(),
        ),
        AdjustmentSettings::brightness_contrast(BrightnessContrast::new(30.0, 20.0).unwrap()),
        AdjustmentSettings::threshold(Threshold::new(128).unwrap()),
        AdjustmentSettings::posterize(Posterize::new(5).unwrap()),
    ]
}

fn complex_document() -> Document {
    complex_document_with_tile_size(Document::DEFAULT_TILE_SIZE)
}

fn complex_document_with_tile_size(tile_size: u32) -> Document {
    // 300 × 200: タイルの端をまたぐ（既定の 128 なら 128・256 の線）。端のタイルは部分
    let mut doc = Document::with_tile_size(300, 200, tile_size).unwrap();
    doc.set_source_budget_bytes(1 << 30).unwrap();
    let mut rng = SplitMix(2026);
    let base = doc.add_layer("base").unwrap();
    fill(&mut doc, base, Channel::Color, &mut rng);
    let opacities = [0.35, 0.7, 1.0, 0.999, 0.05];
    let adjust = adjustments();
    let mut previous = base;
    let mut kept = Vec::new();
    for (i, mode) in BlendMode::LAYER_MODES.into_iter().enumerate() {
        let l = doc.add_layer(&format!("m{i}")).unwrap();
        fill(&mut doc, l, Channel::Color, &mut rng);
        doc.set_layer_blend_mode(l, mode).unwrap();
        doc.set_layer_opacity(l, opacities[i % opacities.len()], false)
            .unwrap();
        if i % 3 == 1 {
            fill_mask(&mut doc, l, &mut rng);
        }
        if i % 4 == 2 {
            doc.set_layer_clipping(l, true).unwrap();
        }
        // Normal チャンネルにも内容（Normal と Overlay で重ねる）
        if i % 5 == 0 {
            doc.set_channel_enabled(l, Channel::Normal, true).unwrap();
            fill(&mut doc, l, Channel::Normal, &mut rng);
            let nm = if i % 10 == 0 {
                BlendMode::Overlay
            } else {
                BlendMode::Normal
            };
            doc.set_channel_blend_mode(l, Channel::Normal, Some(nm))
                .unwrap();
        }
        if i % 3 == 2 {
            let a = doc
                .add_adjustment_layer(
                    &format!("a{i}"),
                    adjust[(i / 3) % adjust.len()].clone(),
                    None,
                    None,
                )
                .unwrap();
            doc.set_layer_opacity(a, 0.8, false).unwrap();
            let am = [
                BlendMode::Normal,
                BlendMode::Overlay,
                BlendMode::Hue,
                BlendMode::Multiply,
            ][(i / 3) % 4];
            doc.set_layer_blend_mode(a, am).unwrap();
            if i % 2 == 0 {
                fill_mask(&mut doc, a, &mut rng);
            }
            kept.push(a);
        }
        kept.push(l);
        previous = l;
    }
    let _ = previous;
    // 通過のグループ（不透明度 0.6 でフェード）と、独立のグループ（Normal）の中にクリッピングの組
    let n = kept.len();
    let g1 = doc.group_layers(&kept[n - 4..n - 1], "G1").unwrap();
    doc.set_layer_opacity(g1, 0.6, false).unwrap();
    let g2 = doc.group_layers(&kept[n - 9..n - 6], "G2").unwrap();
    doc.set_layer_blend_mode(g2, BlendMode::Normal).unwrap();
    doc.set_layer_opacity(g2, 0.85, false).unwrap();
    let fillcolor = doc
        .add_fill_layer("f", &[(Channel::Color, Rgba8::new(30, 90, 200, 255))], None)
        .unwrap();
    doc.set_layer_blend_mode(fillcolor, BlendMode::Multiply)
        .unwrap();
    doc.set_layer_opacity(fillcolor, 0.25, false).unwrap();
    doc
}

#[test]
fn the_tile_path_matches_the_pixel_reference_for_every_mode_and_layer_kind() {
    let doc = complex_document();
    let rects = [
        doc.bounds(),
        Rect::new(37, 11, 251, 177), // タイルの境をまたぐ
        Rect::new(255, 0, 1, 200),
        Rect::new(0, 199, 300, 1),
    ];
    for ch in [Channel::Color, Channel::Normal] {
        for rect in rects {
            let got = doc.composite_channel(ch, rect).unwrap();
            for ry in 0..rect.height {
                for rx in 0..rect.width {
                    let (x, y) = (rect.x + rx, rect.y + ry);
                    // composite_channel の行は下から（BottomUp）。composite_pixel の座標系と同じ
                    let i = ((ry * rect.width + rx) * 4) as usize;
                    let want = doc.composite_pixel(ch, x, y).unwrap().to_array();
                    assert_eq!(
                        &got[i..i + 4],
                        &want,
                        "{ch:?} {rect:?} ({x},{y}) 道 {}",
                        yolu_core::blend::simd_level_name()
                    );
                }
            }
        }
    }
}

/// 歩幅つきの粗い合成（読み元の刻みが 4 より大きい。行は 64 画素ずつ詰めて行の核へ渡る）の画素が、全体の合成のその位置の画素と同じバイトか。
/// タイルが 256 なら歩幅 2 の行は 128 画素で、詰める単位（64 画素）をまたぎ（マスクの読みも 64 画素目から続く）、端のタイルは部分になる。
#[test]
fn a_coarse_composite_matches_the_pixel_reference_at_the_sample_points() {
    for tile_size in [128, 256] {
        let doc = complex_document_with_tile_size(tile_size);
        let coords: Vec<TileCoord> = doc.canvas_tiles().collect();
        for ch in [Channel::Color, Channel::Normal] {
            for stride in [1u32, 2, 4, 8, 16, 64] {
                for t in doc.composite_coarse_tiles(ch, &coords, stride).unwrap() {
                    let (w, h) = t.size();
                    for j in 0..h {
                        for i in 0..w {
                            let (x, y) = (t.rect.x + i * stride, t.rect.y + j * stride);
                            let want = doc.composite_pixel(ch, x, y).unwrap().to_array();
                            let at = ((j * w + i) * 4) as usize;
                            assert_eq!(
                                &t.pixels[at..at + 4],
                                &want,
                                "{ch:?} タイル {tile_size} 歩幅 {stride} ({x},{y}) 道 {}",
                                yolu_core::blend::simd_level_name()
                            );
                        }
                    }
                }
            }
        }
    }
}

/// 表示に寄与するレイヤーの結合は、タイルの経路（行の核）で焼く。結果のレイヤーの画素が、結合前の画素ごとの参照（`composite_pixel`）と同じバイトか
/// （アルファ 0 の画素は RGB も 0 に揃える）。端のタイル（部分）も含める。
#[test]
fn merging_visible_layers_bakes_the_pixel_reference() {
    for tile_size in [128, 256] {
        let mut doc = complex_document_with_tile_size(tile_size);
        let (w, h) = (doc.width(), doc.height());
        let channels = [Channel::Color, Channel::Normal];
        let mut want = Vec::new();
        for ch in channels {
            for y in 0..h {
                for x in 0..w {
                    let p = doc.composite_pixel(ch, x, y).unwrap();
                    want.push(if p.a == 0 { Rgba8::TRANSPARENT } else { p });
                }
            }
        }
        let report = doc.merge_visible("merged", 255).unwrap();
        let merged = doc.layer(report.result_id).expect("結合の結果");
        let mut at = 0;
        for ch in channels {
            for y in 0..h {
                for x in 0..w {
                    assert_eq!(
                        merged.pixel(ch, x, y).unwrap(),
                        want[at],
                        "{ch:?} タイル {tile_size} ({x},{y}) 道 {}",
                        yolu_core::blend::simd_level_name()
                    );
                    at += 1;
                }
            }
        }
    }
}

/// グループの結合は、グループの中身だけを透明から重ねたものを、タイルの経路で焼く。結果のレイヤーの画素が、ほかのレイヤーを隠した文書の画素ごとの参照と
/// 同じバイトか（全モード・マスク・クリッピングの組を含むグループ）。
#[test]
fn merging_a_group_bakes_the_pixel_reference_of_its_children() {
    let mut doc = Document::new(300, 200).unwrap();
    doc.set_source_budget_bytes(1 << 30).unwrap();
    let mut rng = SplitMix(77);
    let under = doc.add_layer("under").unwrap();
    fill(&mut doc, under, Channel::Color, &mut rng);
    let opacities = [0.35, 0.7, 1.0, 0.999];
    let mut members = Vec::new();
    for (i, mode) in BlendMode::LAYER_MODES.into_iter().enumerate() {
        let l = doc.add_layer(&format!("m{i}")).unwrap();
        fill(&mut doc, l, Channel::Color, &mut rng);
        doc.set_layer_blend_mode(l, mode).unwrap();
        doc.set_layer_opacity(l, opacities[i % opacities.len()], false)
            .unwrap();
        if i % 3 == 1 {
            fill_mask(&mut doc, l, &mut rng);
        }
        if i % 4 == 2 {
            doc.set_layer_clipping(l, true).unwrap();
        }
        members.push(l);
    }
    let group = doc.group_layers(&members, "G").unwrap();
    doc.set_layer_blend_mode(group, BlendMode::Normal).unwrap();
    // 参照: 下のレイヤーを隠すと、見えるのはグループの中身だけ
    doc.set_layer_visible(under, false).unwrap();
    let (w, h) = (doc.width(), doc.height());
    let mut want = Vec::new();
    for y in 0..h {
        for x in 0..w {
            let p = doc.composite_pixel(Channel::Color, x, y).unwrap();
            want.push(if p.a == 0 { Rgba8::TRANSPARENT } else { p });
        }
    }
    doc.set_layer_visible(under, true).unwrap();
    let report = doc.merge_group(group, 255).unwrap();
    let merged = doc.layer(report.result_id).expect("結合の結果");
    for y in 0..h {
        for x in 0..w {
            assert_eq!(
                merged.pixel(Channel::Color, x, y).unwrap(),
                want[(y * w + x) as usize],
                "({x},{y}) 道 {}",
                yolu_core::blend::simd_level_name()
            );
        }
    }
}
