//! 効果（フィルターのスタック・Generator・Anchor・塗りつぶしの画像と投影・グラデーション）の正本と core の行き来。
//! 正解は C# の実際の書き手（`tools/io-fixtures/EffectFixture.cs`）が作った正本と、同じ文書の層ごとの評価した出力・入力。
//! 全チャンネルの合成（`*.composite`）は、合成の式が f32 の core の式になってから core で撮り直した（`YOLU_GOLDEN_UPDATE=1`）。
use yolu_core::generator::{MapKind, MapState};
use yolu_core::{
    Channel, Document, EffectInputs, ImageId, ImageInput, MapInput, ModelFrame, Rect, Rgba8,
};
use yolu_io::NativeDocument;

pub const FIXTURES: [&str; 5] = [
    "effects-filters",
    "effects-generators",
    "effects-anchors",
    "effects-fills",
    "effects-paths",
];

pub fn read(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/tests/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

struct Cursor<'a>(&'a [u8], usize);
impl Cursor<'_> {
    fn take(&mut self, n: usize) -> &[u8] {
        let s = &self.0[self.1..self.1 + n];
        self.1 += n;
        s
    }
    fn i32(&mut self) -> i32 {
        i32::from_le_bytes(self.take(4).try_into().unwrap())
    }
    fn f64(&mut self) -> f64 {
        f64::from_le_bytes(self.take(8).try_into().unwrap())
    }
    fn u8(&mut self) -> u8 {
        self.take(1)[0]
    }
}

/// C# の Guid のバイト列から、正本の ID と同じ u128（core_bridge と同じ並べ替え）。
pub fn image_id(mut guid: [u8; 16]) -> ImageId {
    guid[..4].reverse();
    guid[4..6].reverse();
    guid[6..8].reverse();
    ImageId(u128::from_be_bytes(guid))
}

/// `EffectFixture.WriteInputs` が書いた入力の写し。
pub fn inputs() -> EffectInputs {
    inputs_with_images().0
}

/// 入力と、その画像の (ID, 幅, 高さ)。
pub fn inputs_with_images() -> (EffectInputs, Vec<(ImageId, u32, u32)>) {
    let bytes = read("effects-inputs.bin");
    let mut c = Cursor(&bytes, 0);
    let mut inputs = EffectInputs::new();
    let mut images = Vec::new();
    for _ in 0..c.i32() {
        let kind = [
            MapKind::WorldNormal,
            MapKind::Position,
            MapKind::AmbientOcclusion,
            MapKind::Curvature,
            MapKind::Thickness,
            MapKind::TangentNormal,
            MapKind::Height,
            MapKind::Id,
            MapKind::BentNormal,
            MapKind::Opacity,
        ][c.i32() as usize];
        let (w, h) = (c.i32() as u32, c.i32() as u32);
        let key = String::from_utf8(c.take(64).to_vec()).unwrap();
        let min = [c.f64(), c.f64(), c.f64()];
        let max = [c.f64(), c.f64(), c.f64()];
        assert_eq!(c.u8(), 0);
        let n = (w * h) as usize * kind.channels();
        let data: Vec<u16> = (0..n)
            .map(|_| u16::from_le_bytes(c.take(2).try_into().unwrap()))
            .collect();
        let coverage = c.take((w * h) as usize).to_vec();
        inputs = inputs
            .with_map(
                MapInput::new(
                    kind,
                    w,
                    h,
                    data,
                    coverage,
                    min,
                    max,
                    &key,
                    MapState::Current,
                )
                .unwrap(),
            )
            .unwrap();
    }
    let frame = [
        c.f64(),
        c.f64(),
        c.f64(),
        c.f64(),
        c.f64(),
        c.f64(),
        c.f64(),
    ];
    inputs = inputs.with_frame(Some(
        ModelFrame::new(
            [frame[0], frame[1], frame[2]],
            [frame[3], frame[4], frame[5], frame[6]],
        )
        .unwrap(),
    ));
    for _ in 0..c.i32() {
        let id: [u8; 16] = c.take(16).try_into().unwrap();
        let (w, h) = (c.i32() as u32, c.i32() as u32);
        let space = match c.u8() {
            0 => yolu_core::ImageColorSpace::Unspecified,
            1 => yolu_core::ImageColorSpace::Srgb,
            _ => yolu_core::ImageColorSpace::Linear,
        };
        let pixels = c.take((w * h * 4) as usize).to_vec();
        images.push((image_id(id), w, h));
        inputs = inputs.with_image(image_id(id), ImageInput::new(w, h, pixels, space).unwrap());
    }
    assert_eq!(c.1, bytes.len());
    (inputs, images)
}

/// C# の `EffectFixture.Save` と同じ並び: 全チャンネル（番号の順）の合成、続けて Normal のファイル出力。
pub fn composites(doc: &Document) -> Vec<u8> {
    let rect = Rect::new(0, 0, doc.width(), doc.height());
    let mut out = Vec::new();
    for c in Channel::ALL {
        out.extend(doc.composite_channel(c, rect).unwrap());
    }
    out.extend(doc.normal_file_output(u64::MAX).unwrap());
    out
}

/// 層ごとの評価した出力（`EffectFixture.LayerOutputs` と同じ並び）。(見出し, 旗に続く画素) の並び。
pub fn layer_outputs(doc: &Document) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    for (i, layer) in doc.layers().iter().enumerate() {
        for c in Channel::ALL {
            let has = matches!(
                layer.kind(),
                yolu_core::LayerKind::Raster | yolu_core::LayerKind::Fill
            ) && layer.has_evaluated_output(c);
            let mut bytes = vec![u8::from(has)];
            if has {
                for y in 0..doc.height() {
                    for x in 0..doc.width() {
                        bytes.extend(
                            doc.layer_output_pixel(layer.id(), c, x, y)
                                .unwrap()
                                .to_array(),
                        );
                    }
                }
            }
            out.push((format!("層 {i}「{}」{c:?}", layer.name()), bytes));
        }
        let mask = layer.mask().is_some_and(|m| m.has_active_filters());
        let mut bytes = vec![u8::from(mask)];
        if mask {
            for y in 0..doc.height() {
                for x in 0..doc.width() {
                    bytes.push(doc.mask_output_hide(layer.id(), x, y).unwrap());
                }
            }
        }
        out.push((format!("層 {i}「{}」マスク", layer.name()), bytes));
    }
    out
}

pub fn open(name: &str) -> (NativeDocument, Document) {
    let native = NativeDocument::read(&read(&format!("{name}.utpaint"))).unwrap();
    assert_eq!(native.version(), 21, "{name}");
    assert!(
        native.core_issues().is_empty(),
        "{name}: {:?}",
        native.core_issues()
    );
    let mut core = native.to_core().unwrap();
    core.set_effect_inputs(inputs()).unwrap();
    (native, core)
}

#[test]
fn csharp_effect_documents_roundtrip_byte_for_byte() {
    for name in FIXTURES {
        let original = read(&format!("{name}.utpaint"));
        let (native, core) = open(name);
        assert_eq!(native.to_bytes(), original);
        assert_eq!(
            core.undo_count(),
            0,
            "{name}: 読み込みは Undo の履歴に残さない"
        );
        assert_eq!(
            NativeDocument::from_core(&core).unwrap().to_bytes(),
            original,
            "{name}: C# の正本を core にして書き戻すとバイト一致"
        );
    }
}

#[test]
fn effect_documents_composite_every_channel_like_the_recorded_bytes() {
    for name in FIXTURES {
        let (_, core) = open(name);
        let got = composites(&core);
        let want = read(&format!("{name}.composite"));
        if got != want && std::env::var_os("YOLU_GOLDEN_UPDATE").is_some() {
            let path = format!(
                "{}/tests/fixtures/{name}.composite",
                env!("CARGO_MANIFEST_DIR")
            );
            std::fs::write(path, &got).unwrap();
            continue;
        }
        assert_eq!(got.len(), want.len(), "{name}");
        if got != want {
            let at = got.iter().zip(&want).position(|(a, b)| a != b).unwrap();
            let plane = core.width() as usize * core.height() as usize * 4;
            panic!(
                "{name}: 合成が正解と違う: チャンネル順の {} 枚目の画素 {} ({} 個違う)",
                at / plane,
                (at % plane) / 4,
                got.iter().zip(&want).filter(|(a, b)| a != b).count()
            );
        }
    }
}

/// 層ごとの評価した出力（`layer_outputs`）が、C# の `LayerOutputs` の記録と同じか。違う所の説明の一覧（空なら一致）。
pub fn layer_output_mismatches(core: &Document, want: &[u8]) -> Vec<String> {
    let mut at = 0;
    let (w, h) = (core.width() as usize, core.height() as usize);
    let mut bad = Vec::new();
    for (label, got) in layer_outputs(core) {
        let len = got.len();
        let expected = &want[at..at + len.min(want.len() - at)];
        if expected != &got[..] {
            let first = got.iter().zip(expected).position(|(a, b)| a != b);
            let wrong = got.iter().zip(expected).filter(|(a, b)| a != b).count();
            if std::env::var("EFFECT_DEBUG").is_ok() && got.len() > 1 && len == expected.len() {
                for (i, (a, b)) in got[1..].chunks(4).zip(expected[1..].chunks(4)).enumerate() {
                    if a != b {
                        eprintln!("  {label} ({}, {}) 得 {a:?} 正 {b:?}", i % w, i / w);
                    }
                }
            }
            bad.push(format!(
                "{label}: 旗 {} と {}、違う {wrong} バイト、最初 {:?}（幅 {w} 高さ {h}）",
                got[0],
                expected[0],
                first.map(|p| ((p - 1) / 4 % w, (p - 1) / 4 / w))
            ));
        }
        // 旗が違うと長さが違うので、C# の旗に合わせて進める
        at += 1 + if expected[0] == 1 {
            if label.ends_with("マスク") {
                w * h
            } else {
                w * h * 4
            }
        } else {
            0
        };
    }
    if at != want.len() {
        bad.push(format!("記録の長さが違う: 読んだ {at} と {}", want.len()));
    }
    bad
}

#[test]
fn csharp_effect_documents_evaluate_every_layer_like_csharp() {
    for name in FIXTURES {
        let (_, core) = open(name);
        let bad = layer_output_mismatches(&core, &read(&format!("{name}.layers")));
        assert!(
            bad.is_empty(),
            "{name}: 層の評価した出力が C# と違う:\n{}",
            bad.join("\n")
        );
    }
}

/// 版 9〜20 の旧い正本（効果は版ごとに足されたものだけ）は、C# の読み手と同じ意味で core になり、版 21 へ移すと C# が書き直したバイト列と同じ。
#[test]
fn legacy_versions_with_effects_read_and_migrate_like_csharp() {
    for v in 9..=20 {
        let name = format!("effects-legacy-v{v}");
        let native = NativeDocument::read(&read(&format!("{name}.utpaint"))).unwrap();
        assert_eq!(native.version(), v, "{name}");
        assert!(
            native.core_issues().is_empty(),
            "{name}: {:?}",
            native.core_issues()
        );
        let mut core = native.to_core().unwrap();
        core.set_effect_inputs(inputs()).unwrap();
        assert_eq!(
            NativeDocument::from_core(&core).unwrap().to_bytes(),
            read(&format!("{name}.v21")),
            "{name}: 版 21 へ移した正本が C# の書き直しと同じバイト列"
        );
        assert_eq!(
            composites(&core),
            read(&format!("{name}.composite")),
            "{name}: 合成が C# と同じ"
        );
        // 効果が版ごとに増える（読めただけで、効果が空になっていない）
        let layer = &core.layers()[0];
        let stages = layer.filters().len();
        let expected = match v {
            9 | 10 => 1,
            11 | 12 => 2,
            13 | 14 => 3,
            _ => 4,
        };
        assert_eq!(stages, expected, "{name}");
        assert_eq!(layer.mask().map_or(0, |m| m.filters().len()), 1, "{name}");
        assert_eq!(core.anchors().len(), usize::from(v >= 20), "{name}");
    }
}

/// ブロックの大きさは結果を変えない（正解と全バイト一致のまま、評価のブロックを替える）。
#[test]
fn block_size_does_not_change_the_pixels() {
    for name in FIXTURES {
        for block in [8, 16, 24, 40, 100, 256, 4096] {
            let (_, mut core) = open(name);
            core.set_filter_block_pixels(block).unwrap();
            assert!(
                composites(&core) == read(&format!("{name}.composite")),
                "{name} ブロック {block}"
            );
        }
    }
}

/// 壊れた正本は断る（通ったものも、core へ変えるときに落ちない）。
#[test]
fn truncated_and_damaged_effect_documents_never_panic() {
    for name in [
        "effects-filters",
        "effects-generators",
        "effects-anchors",
        "effects-fills",
        "effects-legacy-v20",
    ] {
        let original = read(&format!("{name}.utpaint"));
        // どの長さで切っても読めない（先頭から数えて足りない）
        for n in (0..original.len()).step_by(37).chain([original.len() - 1]) {
            assert!(
                NativeDocument::read(&original[..n]).is_err(),
                "{name}: {n} バイトで切って読めた"
            );
        }
        // 1 バイトずつ壊す。読めたものは、core へ変えるか断るかで、落ちない
        let mut rng = 0x9E3779B97F4A7C15u64;
        for _ in 0..600 {
            rng = rng
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let at = (rng >> 33) as usize % original.len();
            let mut damaged = original.clone();
            damaged[at] ^= 1 << ((rng >> 20) % 8);
            if let Ok(native) = NativeDocument::read(&damaged) {
                if native.core_issues().is_empty() {
                    let _ = native.to_core();
                }
            }
        }
    }
}

/// プロジェクトの画像リソースが、効果の入力になる（ID・ハッシュ・向き・色空間）。
#[test]
fn project_images_become_effect_inputs() {
    use yolu_io::Project;
    for name in ["format5.ylp", "format6.ylp"] {
        let project = Project::read(&read(name)).unwrap();
        let images = project.image_inputs().unwrap();
        let resources: Vec<_> = project
            .resources()
            .iter()
            .filter(|r| r.kind == "image")
            .collect();
        assert!(name != "format5.ylp" || !images.is_empty(), "{name}");
        assert_eq!(images.len(), resources.len(), "{name}");
        for ((id, image), r) in images.iter().zip(&resources) {
            assert_eq!(
                image.hash, r.content,
                "{name}: 索引の content と同じハッシュ"
            );
            assert_eq!(format!("{:032x}", id.0), r.id.replace('-', ""), "{name}");
        }
        // 文書へ渡せる
        let mut doc = Document::with_tile_size(8, 8, 8).unwrap();
        let mut inputs = EffectInputs::new();
        for (id, image) in images {
            inputs = inputs.with_image(id, image);
        }
        doc.set_effect_inputs(inputs).unwrap();
    }
}

/// 2D のパスは、文書の `set_canvas_path` が、C# の `SetCanvasPath` と同じ画素を描く。パスと画素は 1 回の Undo で入れ替わり、
/// パスの層には手で描けない。パスを外す（ラスタライズ）と画素だけが残って描ける。
#[test]
fn canvas_paths_render_like_csharp_and_undo_with_their_pixels() {
    use yolu_core::LayerPath;
    let (_, fixture) = open("effects-paths");
    let mut tested = 0;
    for layer in fixture.layers() {
        let Some(LayerPath::Canvas(path)) = layer.path() else {
            continue;
        };
        let mut doc =
            Document::with_tile_size(fixture.width(), fixture.height(), fixture.tile_size())
                .unwrap();
        let id = doc.add_layer("x").unwrap();
        let before = whole(&doc);
        let steps = doc.undo_count();
        doc.set_canvas_path(id, path.clone()).unwrap();
        assert_eq!(doc.undo_count(), steps + 1, "パスと画素は 1 回の Undo");
        for c in path.material.as_ref().map_or(vec![path.channel], |m| {
            m.iter().map(|p| p.channel).collect()
        }) {
            assert_eq!(
                doc.layer(id).unwrap().surface(c).unwrap().to_canvas_bytes(),
                layer.surface(c).unwrap().to_canvas_bytes(),
                "{}: {c:?} の画素が C# と同じ",
                layer.name()
            );
        }
        assert!(doc.layer(id).unwrap().path().is_some());
        // 手で描けない
        let brush = yolu_core::BrushSettings::default();
        assert!(doc.begin_stroke_in(id, path.channel, &brush).is_err());
        assert!(doc
            .fill(id, path.channel, Rgba8::new(1, 2, 3, 255), 1.0, None, false)
            .is_err());
        // 描くチャンネルを無効にできないのは、組（material）を持たないパスだけ（C# の `Path.Material == null`）
        assert_eq!(
            doc.set_channel_enabled(id, path.channel, false).is_err(),
            path.material.is_none(),
            "{}",
            layer.name()
        );
        if path.material.is_some() {
            doc.undo().unwrap(); // 無効にした 1 回を戻す（このあとの Undo でパスと画素が戻る）
        }
        // Undo でパスも画素も戻る
        doc.undo().unwrap();
        assert!(doc.layer(id).unwrap().path().is_none());
        assert_eq!(whole(&doc), before);
        doc.redo().unwrap();
        // ラスタライズで、画素だけが残る
        let pixels = whole(&doc);
        doc.rasterize(id).unwrap();
        assert!(doc.layer(id).unwrap().path().is_none());
        assert_eq!(whole(&doc), pixels);
        let mut s = doc.begin_stroke_in(id, path.channel, &brush).unwrap();
        s.add_point(&mut doc, 3.0, 3.0, 1.0, yolu_core::glam::DVec2::ZERO)
            .unwrap();
        doc.end_stroke(s).unwrap();
        doc.undo().unwrap(); // 描いた 1 筆
        doc.undo().unwrap(); // ラスタライズ
        assert!(
            doc.layer(id).unwrap().path().is_some(),
            "ラスタライズも 1 回の Undo"
        );
        tested += 1;
    }
    assert_eq!(tested, 2);
}

fn whole(doc: &Document) -> Vec<u8> {
    composites(doc)
}

// ───────── Rust の編集 API で作った文書 ─────────

fn paint_pixels(doc: &mut Document, id: yolu_core::LayerId, channel: Channel, seed: u32) {
    for y in 0..doc.height() {
        for x in 0..doc.width() {
            if (x + 2 * y + seed).is_multiple_of(3) {
                continue;
            }
            let v = (x * 37 + y * 19 + seed * 57) as u8;
            let px = Rgba8::new(v, v.wrapping_mul(3), v.wrapping_add(90), v | 1);
            doc.set_channel_pixel(id, channel, x, y, px).unwrap();
        }
    }
}

fn mask_strokes(doc: &mut Document, id: yolu_core::LayerId) {
    let mut s = doc
        .begin_mask_stroke(id, &yolu_core::BrushSettings::default())
        .unwrap();
    for (x, y) in [(6.0, 6.0), (20.0, 12.0), (34.0, 20.0)] {
        s.add_point(doc, x, y, 1.0, yolu_core::glam::DVec2::ZERO)
            .unwrap();
    }
    doc.end_stroke(s).unwrap();
}

/// Rust の編集 API（add_filter・move_filter・remove_filter・Undo・add_anchor・set_generator_anchor・set_fill_image・set_fill_projection・
/// set_fill_gradient・set_canvas_path）だけで、効果を一通り付けた文書。ID は固定。C# の書き手が作った正本には現れない、Rust だけが作れる状態
/// （参照が未選択の Anchor Generator、値の無い塗りつぶしに付けたグラデーション、編集で入れ替わった段の並び、Undo で戻した段）を含む。
/// 画布は C# の入力（メッシュマップ・画像）と同じ 41×27。
pub fn edited_effects_document() -> Document {
    let doc = edited_effects_with_history();
    let ids: Vec<yolu_core::LayerId> = (0..doc.layers().len())
        .map(|i| yolu_core::LayerId(0x7000 + i as u128))
        .collect();
    doc.with_persistent_ids(0x7777, &ids).unwrap()
}

/// `edited_effects_document` の、層の ID を固定する前の文書（Undo の履歴が残っている）。
pub fn edited_effects_with_history() -> Document {
    use yolu_core::fill_image::{Placement, Projection, ProjectionMode};
    use yolu_core::generator::{self, anchor::ReadMode, Ramp, Settings};
    use yolu_core::paths::{CanvasPath, CanvasPoint, PathBrush};
    use yolu_core::{
        AnchorId, AnchorPlacement, BlendMode, BrushSettings, EffectSettings, FilterId, FilterSpec,
        FilterTarget,
    };
    let (inputs, images) = inputs_with_images();
    let image = |w: u32, h: u32| {
        images
            .iter()
            .find(|i| (i.1, i.2) == (w, h))
            .expect("入力の画像")
            .0
    };
    let (stripes, shape, grey) = (image(8, 6), image(6, 6), image(11, 9));
    let fid = |n: u128| FilterId(0xF000 + n);
    let aid = |n: u128| AnchorId(0xA000 + n);
    let ramp = || {
        let mut g = Settings::new(generator::Kind::ShapeGradient);
        g.ramp = Some(Ramp::default());
        g.blend = generator::Blend::Replace;
        g
    };
    let mut doc = Document::with_tile_size(41, 27, 8).unwrap();
    doc.set_effect_inputs(inputs).unwrap();

    // 土台: 段の並びを編集で入れ替え、外した段を Undo で戻す。マスクと、層・マスクの Anchor
    let base = doc.add_layer("土台").unwrap();
    for (c, seed) in [
        (Channel::Color, 1),
        (Channel::Roughness, 2),
        (Channel::Height, 3),
    ] {
        paint_pixels(&mut doc, base, c, seed);
    }
    let on =
        |doc: &mut Document, n: u128, s: EffectSettings, channels: &[Channel], strength: f64| {
            doc.add_filter(
                base,
                FilterTarget::Content,
                FilterSpec::new(s)
                    .channels(channels)
                    .strength(strength)
                    .with_id(fid(n)),
            )
            .unwrap();
        };
    on(
        &mut doc,
        1,
        EffectSettings::blur(3),
        &[Channel::Color, Channel::Roughness],
        0.8,
    );
    on(
        &mut doc,
        2,
        EffectSettings::sharpen(2, 0.75, 17),
        &[Channel::Color],
        1.0,
    );
    on(
        &mut doc,
        3,
        EffectSettings::noise(0.3, -123, true),
        &[Channel::Color, Channel::Roughness],
        1.0,
    );
    on(
        &mut doc,
        4,
        EffectSettings::levels(0.1, 0.9, 1.7, 0.2, 0.8),
        &[Channel::Color],
        1.0,
    );
    on(
        &mut doc,
        5,
        EffectSettings::invert(),
        &[Channel::Color],
        1.0,
    );
    on(
        &mut doc,
        6,
        EffectSettings::normalize(),
        &[Channel::Color],
        0.6,
    );
    doc.set_filter_enabled(base, fid(5), false).unwrap();
    doc.move_filter(base, fid(3), 5).unwrap();
    doc.move_filter(base, fid(6), 0).unwrap();
    doc.remove_filter(base, fid(2)).unwrap();
    doc.undo().unwrap(); // 外した段を戻す（元の位置へ）
    doc.add_layer_mask(base).unwrap();
    mask_strokes(&mut doc, base);
    for (n, s, strength) in [
        (11, EffectSettings::noise(0.2, 57, true), 1.0),
        (12, EffectSettings::levels(0.05, 0.95, 1.2, 0.1, 1.0), 1.0),
        (13, EffectSettings::blur(2), 1.0),
        (14, EffectSettings::invert(), 0.5),
    ] {
        doc.add_filter(
            base,
            FilterTarget::Mask,
            FilterSpec::new(s).strength(strength).with_id(fid(n)),
        )
        .unwrap();
    }
    doc.set_layer_opacity(base, 0.85, false).unwrap();
    doc.set_layer_mask_density(base, 0.8, false).unwrap();
    doc.add_anchor(base, AnchorPlacement::Layer, Some("土台"), Some(aid(1)))
        .unwrap();
    doc.add_anchor(
        base,
        AnchorPlacement::Mask,
        Some("土台のマスク"),
        Some(aid(2)),
    )
    .unwrap();

    // 中: Anchor
    let mid = doc.add_layer("中").unwrap();
    paint_pixels(&mut doc, mid, Channel::Height, 4);
    doc.set_layer_blend_mode(mid, BlendMode::Multiply).unwrap();
    doc.set_layer_opacity(mid, 0.6, false).unwrap();
    doc.add_anchor(mid, AnchorPlacement::Layer, Some("中の層"), Some(aid(3)))
        .unwrap();

    // 読む塗りつぶし: 土台の Height を値で、Color を被覆で読み、マスクは土台のマスクを反転して読む。参照が未選択の Anchor Generator も持つ
    let reader = doc
        .add_fill_layer(
            "読む",
            &[
                (Channel::Height, Rgba8::new(128, 128, 128, 255)),
                (Channel::Color, Rgba8::new(60, 160, 220, 255)),
            ],
            None,
        )
        .unwrap();
    let anchor_stage = |doc: &mut Document,
                        target: FilterTarget,
                        n: u128,
                        channels: &[Channel],
                        blend: generator::Blend,
                        strength: f64,
                        invert: bool| {
        let mut g = Settings::new(generator::Kind::Anchor);
        g.blend = blend;
        g.invert = invert;
        let mut spec = FilterSpec::new(EffectSettings::generator(g))
            .strength(strength)
            .with_id(fid(n));
        if !channels.is_empty() {
            spec = spec.channels(channels);
        }
        doc.add_filter(reader, target, spec).unwrap();
    };
    anchor_stage(
        &mut doc,
        FilterTarget::Content,
        21,
        &[Channel::Height],
        generator::Blend::Multiply,
        1.0,
        false,
    );
    doc.set_generator_anchor(
        reader,
        fid(21),
        Some(aid(1)),
        Channel::Height,
        ReadMode::Value,
        false,
    )
    .unwrap();
    anchor_stage(
        &mut doc,
        FilterTarget::Content,
        22,
        &[Channel::Color],
        generator::Blend::Replace,
        0.8,
        false,
    );
    doc.set_generator_anchor(
        reader,
        fid(22),
        Some(aid(1)),
        Channel::Color,
        ReadMode::Coverage,
        false,
    )
    .unwrap();
    doc.add_filter(
        reader,
        FilterTarget::Content,
        FilterSpec::new(EffectSettings::blur(3))
            .channels(&[Channel::Color])
            .with_id(fid(23)),
    )
    .unwrap();
    anchor_stage(
        &mut doc,
        FilterTarget::Content,
        24,
        &[Channel::Height],
        generator::Blend::Min,
        1.0,
        false,
    ); // 参照が未選択
    doc.add_layer_mask(reader).unwrap();
    anchor_stage(
        &mut doc,
        FilterTarget::Mask,
        25,
        &[],
        generator::Blend::Replace,
        1.0,
        true,
    );
    doc.set_generator_anchor(
        reader,
        fid(25),
        Some(aid(2)),
        Channel::Height,
        ReadMode::Value,
        false,
    )
    .unwrap();
    doc.add_anchor(reader, AnchorPlacement::Layer, Some("読む層"), Some(aid(4)))
        .unwrap();

    // Generator の段（入力のマップを読む）と、マスクの Generator
    let gen = doc.add_layer("Generator").unwrap();
    paint_pixels(&mut doc, gen, Channel::Height, 5);
    paint_pixels(&mut doc, gen, Channel::Color, 6);
    let mut position = Settings::new(generator::Kind::PositionGradient);
    position.axis = 2;
    let mut edge = Settings::new(generator::Kind::EdgeWear);
    edge.noise_amount = 0.0;
    for (n, g, channels) in [
        (31u128, position, vec![Channel::Height]),
        (32, edge, vec![Channel::Height]),
        (33, ramp(), vec![Channel::Color]),
    ] {
        doc.add_filter(
            gen,
            FilterTarget::Content,
            FilterSpec::new(EffectSettings::generator(g))
                .channels(&channels)
                .with_id(fid(n)),
        )
        .unwrap();
    }
    doc.add_layer_mask(gen).unwrap();
    doc.add_filter(
        gen,
        FilterTarget::Mask,
        FilterSpec::new(EffectSettings::generator(ramp())).with_id(fid(34)),
    )
    .unwrap();

    // 塗りつぶしの画像と投影、デカール、グラデーション
    let projected = doc
        .add_fill_layer(
            "画像",
            &[
                (Channel::Color, Rgba8::new(10, 200, 30, 255)),
                (Channel::Roughness, Rgba8::new(90, 90, 90, 255)),
            ],
            None,
        )
        .unwrap();
    doc.set_fill_image(projected, Channel::Color, Some(stripes))
        .unwrap();
    doc.set_fill_image(projected, Channel::Roughness, Some(grey))
        .unwrap();
    doc.set_fill_projection(
        projected,
        Projection {
            mode: ProjectionMode::Triplanar,
            tiles: [2.0, 2.0],
            blend_width: 0.4,
            placement: Placement {
                center: [0.1, 0.2, -0.3],
                rotation: [10.0, 20.0, 30.0],
                size: [1.5, 2.0, 2.5],
            },
            ..Projection::default()
        },
        false,
    )
    .unwrap();
    doc.set_layer_opacity(projected, 0.5, false).unwrap();
    let decal = doc
        .add_fill_layer(
            "デカール",
            &[
                (Channel::Color, Rgba8::new(220, 40, 40, 255)),
                (Channel::Height, Rgba8::new(255, 255, 255, 255)),
                (Channel::Roughness, Rgba8::new(200, 200, 200, 255)),
            ],
            None,
        )
        .unwrap();
    doc.set_fill_image(decal, Channel::Color, Some(stripes))
        .unwrap();
    doc.set_fill_image(decal, Channel::Height, Some(shape))
        .unwrap();
    doc.set_fill_projection(
        decal,
        Projection {
            mode: ProjectionMode::Decal,
            placement: Placement {
                center: [0.4, 0.3, 0.0],
                rotation: [0.0, 25.0, 0.0],
                size: [2.5, 3.0, 2.0],
            },
            depth_hardness: 0.6,
            backface_angle: 100.0,
            backface_hardness: 0.5,
            ..Projection::default()
        },
        false,
    )
    .unwrap();
    doc.set_fill_gradient(decal, Channel::Roughness, Some(ramp()), false)
        .unwrap();
    // 値の無い塗りつぶしにグラデーション（値は既定になる）。もう 1 つは画像からグラデーションへ替える
    let empty = doc.add_fill_layer("値なし", &[], None).unwrap();
    doc.set_fill_gradient(empty, Channel::Color, Some(ramp()), false)
        .unwrap();
    doc.set_fill_gradient(empty, Channel::Height, Some(ramp()), false)
        .unwrap();
    let swapped = doc
        .add_fill_layer(
            "画像からグラデーション",
            &[(Channel::Color, Rgba8::new(30, 30, 30, 255))],
            None,
        )
        .unwrap();
    doc.set_fill_image(swapped, Channel::Color, Some(stripes))
        .unwrap();
    doc.set_fill_gradient(swapped, Channel::Color, Some(ramp()), false)
        .unwrap();

    // 2D のパス（組）と、そのマスクのフィルター
    let path_layer = doc.add_layer("パス").unwrap();
    doc.set_canvas_path(
        path_layer,
        CanvasPath {
            id: 0x77,
            channel: Channel::Color,
            brush: PathBrush(BrushSettings {
                radius: 3.0,
                spacing: 0.17,
                color: Rgba8::new(201, 37, 89, 219),
                ..BrushSettings::default()
            }),
            points: vec![
                CanvasPoint::new(4.5, 7.25, 0.3).unwrap(),
                CanvasPoint::new(29.5, 21.5, 0.9).unwrap(),
                CanvasPoint::new(37.25, 5.75, 0.55).unwrap(),
            ],
            material: Some(vec![
                yolu_core::paths::ChannelPaint {
                    channel: Channel::Color,
                    color: Rgba8::new(31, 87, 231, 255),
                },
                yolu_core::paths::ChannelPaint {
                    channel: Channel::Roughness,
                    color: Rgba8::new(140, 140, 140, 255),
                },
            ]),
        },
    )
    .unwrap();
    doc.add_layer_mask(path_layer).unwrap();
    mask_strokes(&mut doc, path_layer);
    doc.add_filter(
        path_layer,
        FilterTarget::Mask,
        FilterSpec::new(EffectSettings::blur(2)).with_id(fid(41)),
    )
    .unwrap();
    // グループ（マスクとそのフィルター付き）
    let group = doc.group_layers(&[gen], "グループ").unwrap();
    doc.add_layer_mask(group).unwrap();
    doc.add_filter(
        group,
        FilterTarget::Mask,
        FilterSpec::new(EffectSettings::blur(4)).with_id(fid(42)),
    )
    .unwrap();
    doc.set_layer_mask_inverted(group, true).unwrap();
    doc
}

/// 効果の設定が、2 つの文書で層・マスクごとに同じ（段・Anchor・塗りつぶしの画像・投影・グラデーション・パス・値・順序）。
fn assert_same_effects(a: &Document, b: &Document, what: &str) {
    assert_eq!(a.layers().len(), b.layers().len(), "{what}");
    for (x, y) in a.layers().iter().zip(b.layers()) {
        let name = x.name();
        assert_eq!(x.id(), y.id(), "{what}: {name}");
        assert_eq!(x.kind(), y.kind(), "{what}: {name}");
        assert_eq!(x.parent(), y.parent(), "{what}: {name}");
        assert_eq!(x.filters(), y.filters(), "{what}: {name} の段");
        assert_eq!(x.anchor(), y.anchor(), "{what}: {name} の Anchor");
        assert_eq!(x.path(), y.path(), "{what}: {name} のパス");
        assert_eq!(x.projection(), y.projection(), "{what}: {name} の投影");
        assert_eq!(
            x.fill_values().collect::<Vec<_>>(),
            y.fill_values().collect::<Vec<_>>(),
            "{what}: {name} の値"
        );
        assert_eq!(
            x.fill_images().collect::<Vec<_>>(),
            y.fill_images().collect::<Vec<_>>(),
            "{what}: {name} の画像"
        );
        assert_eq!(
            x.fill_gradients().collect::<Vec<_>>(),
            y.fill_gradients().collect::<Vec<_>>(),
            "{what}: {name} のグラデーション"
        );
        assert_eq!(
            x.mask()
                .map(|m| (m.filters().to_vec(), m.anchor().cloned())),
            y.mask()
                .map(|m| (m.filters().to_vec(), m.anchor().cloned())),
            "{what}: {name} のマスクの段・Anchor"
        );
        assert_eq!(
            x.mask().map(|m| (m.enabled(), m.inverted(), m.density())),
            y.mask().map(|m| (m.enabled(), m.inverted(), m.density())),
            "{what}: {name} のマスク"
        );
    }
    assert_eq!(a.anchors().len(), b.anchors().len(), "{what}");
}

/// 編集 API で効果を一通り付けた文書を、正本にして開き直しても、層・マスクごとの設定と全チャンネルの合成が同じ。正本は書き直しても同じバイト列で、
/// .ylp にして開き直しても同じ。
#[test]
fn a_document_edited_with_every_effect_api_survives_save_and_reopen() {
    let edited = edited_effects_document();
    let (stages, anchors) = (
        edited
            .layers()
            .iter()
            .map(|l| l.filters().len() + l.mask().map_or(0, |m| m.filters().len()))
            .sum::<usize>(),
        edited.anchors().len(),
    );
    assert!(stages >= 20 && anchors == 4, "{stages} {anchors}");
    assert_eq!(
        edited.inactive_effects().len(),
        1,
        "入力のまま通す効果は、参照が未選択の Anchor ジェネレーターだけ: {:?}",
        edited.inactive_effects()
    );
    let native = NativeDocument::from_core(&edited).unwrap();
    assert!(
        native.core_issues().is_empty(),
        "{:?}",
        native.core_issues()
    );
    let bytes = native.to_bytes();
    let mut reopened = NativeDocument::read(&bytes).unwrap().to_core().unwrap();
    reopened.set_effect_inputs(inputs()).unwrap();
    assert_same_effects(&edited, &reopened, "正本で開き直す");
    assert_eq!(
        composites(&edited),
        composites(&reopened),
        "全チャンネルの合成が元と同じ"
    );
    assert_eq!(
        layer_outputs(&edited),
        layer_outputs(&reopened),
        "層ごとの評価した出力が元と同じ"
    );
    assert_eq!(
        NativeDocument::from_core(&reopened).unwrap().to_bytes(),
        bytes,
        "書き直しても同じバイト列"
    );
    // .ylp にして開き直す
    let set = |doc: &Document| yolu_io::SetSpec {
        id: "0f1e2d3c-4b5a-4978-8796-a5b4c3d2e1f0".into(),
        name: "効果".into(),
        material: yolu_io::MaterialRef::Unassigned,
        document: Some(NativeDocument::from_core(doc).unwrap().into()),
        composites: yolu_io::composite_pngs(doc).unwrap(),
    };
    let project = yolu_io::Project::create(
        yolu_io::WriterInfo {
            app: "試験の書き手".into(),
            version: "0.0.1".into(),
            unity: "standalone".into(),
        },
        &[set(&edited)],
        "0f1e2d3c-4b5a-4978-8796-a5b4c3d2e1f0",
    )
    .unwrap();
    let project = yolu_io::Project::read(&project.to_bytes().unwrap()).unwrap();
    let mut from_project = project.sets()[0].document.to_core().unwrap();
    from_project.set_effect_inputs(inputs()).unwrap();
    assert_same_effects(&edited, &from_project, ".ylp で開き直す");
    assert_eq!(composites(&edited), composites(&from_project));
    // Undo の履歴は保存されない（開き直した文書は何も戻せない）
    assert!(!reopened.can_undo());
}

/// 編集 API の途中の状態も往復できる: 編集の 1 つずつ（Undo で戻した状態も）で、正本にして開き直すと同じ。
#[test]
fn every_step_of_editing_effects_round_trips() {
    let mut doc = edited_effects_with_history();
    let mut steps = 0;
    // 最後から順に Undo し、その状態ごとに往復を確かめる。Redo で戻して同じ状態へ
    let mut states = Vec::new();
    loop {
        let native = NativeDocument::from_core(&doc).unwrap();
        states.push(native.to_bytes());
        let mut reopened = native.to_core().unwrap();
        reopened.set_effect_inputs(inputs()).unwrap();
        assert_same_effects(&doc, &reopened, &format!("Undo {steps} 回の状態"));
        if steps % 7 == 0 {
            assert_eq!(composites(&doc), composites(&reopened), "{steps}");
        }
        if !doc.undo().unwrap() {
            break;
        }
        steps += 1;
    }
    assert!(steps >= 25, "{steps}");
    for expected in states.iter().rev().skip(1) {
        assert!(doc.redo().unwrap());
        assert_eq!(
            &NativeDocument::from_core(&doc).unwrap().to_bytes(),
            expected,
            "Redo で同じ状態"
        );
    }
}

/// 編集 API で作った文書の正本（固定の ID で作るので、ファイルと同じバイト列になる）。作り直すときは
/// `YOLU_UPDATE_FIXTURES=1 cargo test -p yolu-io --test ylp effects_bridge::rust_written_effects_fixture_is`、続けて
/// `python3 tools/io-fixtures/generate.py --source <Unity 版> --rust-written-effects` で Unity 版の読み手の記録を取り直す。
#[test]
fn rust_written_effects_fixture_is_what_this_writer_produces() {
    let bytes = NativeDocument::from_core(&edited_effects_document())
        .unwrap()
        .to_bytes();
    let path = format!(
        "{}/tests/fixtures/rust-written-effects-v21.utpaint",
        env!("CARGO_MANIFEST_DIR")
    );
    if std::env::var_os("YOLU_UPDATE_FIXTURES").is_some() {
        std::fs::write(&path, &bytes).unwrap();
    }
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert_eq!(bytes[8..12], 21i32.to_le_bytes());
}

/// 上の正本を Unity 版の読み手（`DocumentBinary`、`Runtime/Core` をそのままコンパイル）に読ませた記録: 読めて、書き直すと同じバイト列になり、
/// 全チャンネルの合成と層ごとの評価した出力が、Rust の評価と全バイト一致する。
#[test]
fn csharp_reads_and_resaves_the_effects_document_this_writer_produces() {
    let edited = edited_effects_document();
    assert_eq!(
        read("rust-written-effects-v21.utpaint"),
        NativeDocument::from_core(&edited).unwrap().to_bytes(),
        "記録した正本が今の書き手の出力と同じ（古くなっていない）"
    );
    let record = String::from_utf8(read("rust-written-effects-v21.unity.txt")).unwrap();
    assert_eq!(
        record.lines().collect::<Vec<_>>(),
        [
            "DocumentBinary.CurrentVersion: 21".to_string(),
            "DocumentBinary.ReadId: OK".to_string(),
            "DocumentBinary.Read: OK".to_string(),
            "DocumentBinary.Write(Read) == input: True".to_string(),
            format!("Layers: {}", edited.layers().len()),
        ]
    );
    let bad = layer_output_mismatches(&edited, &read("rust-written-effects-v21.layers"));
    assert!(
        bad.is_empty(),
        "C# が読んだ文書の層の出力が Rust と違う:\n{}",
        bad.join("\n")
    );
}
