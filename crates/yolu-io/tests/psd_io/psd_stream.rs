//! PSD を実物の大きさで書く: 予算は呼び手（設定）から決まり、レイヤー・マスク・統合画像のチャンネルは RLE（PackBits）で、レイヤーは 1 枚ずつ流して書く。
//! RLE は今の読み手（`read`・`import_copy`・`verify_stream`）で読み直せること、チャンネルごとに圧縮の有無を選ぶこと、流して書いたバイト列が
//! メモリに組んで書いたものと同じであること、予算・ファイルの上限でレイヤーの名前つきで断ること、取消で止まることを固定する。
//! 無圧縮の書き方（Unity 版とバイト一致）は `psd_golden.rs` が固定する。Photoshop・CLIP STUDIO の実物では確かめていない。
use std::io::Cursor;
use std::sync::atomic::AtomicBool;
use yolu_core::{
    AdjustmentSettings, Channel, Document, EffectSettings, FilterSpec, FilterTarget, LayerId,
    Rgba8, TileCoord,
};
use yolu_io::psd::{
    self, Compression, Document as Psd, ExportControl, ExportError, ExportMode, ExportOptions,
    Layer, LayerKind, Limits, Mask, Overrun,
};

const MIB: u64 = 1024 * 1024;

// ───────── ツール ─────────

fn noise(seed: &mut u32) -> u8 {
    *seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
    (*seed >> 24) as u8
}

/// レイヤーの左下の `w` × `h` を、`pixel(x, y)`（core の座標。下から）で塗る。
fn paint(d: &mut Document, layer: LayerId, w: u32, h: u32, pixel: impl Fn(u32, u32) -> [u8; 4]) {
    paint_channel(d, layer, Channel::Color, w, h, pixel)
}
fn paint_channel(
    d: &mut Document,
    layer: LayerId,
    channel: Channel,
    w: u32,
    h: u32,
    pixel: impl Fn(u32, u32) -> [u8; 4],
) {
    let ts = d.tile_size();
    for ty in 0..h.div_ceil(ts) {
        for tx in 0..w.div_ceil(ts) {
            let mut tile = vec![0u8; (ts * ts * 4) as usize];
            for y in 0..ts.min(h - ty * ts) {
                for x in 0..ts.min(w - tx * ts) {
                    let p = pixel(tx * ts + x, ty * ts + y);
                    tile[((y * ts + x) * 4) as usize..][..4].copy_from_slice(&p);
                }
            }
            d.import_tile(layer, channel, TileCoord::new(tx, ty), &tile)
                .unwrap();
        }
    }
}

fn bake() -> ExportOptions {
    ExportOptions::new(Channel::Color, ExportMode::Bake)
}

/// 計画して、`ctl` の予算で流して書く。
fn stream(
    d: &Document,
    ctl: &ExportControl,
    compression: Compression,
) -> Result<(Vec<u8>, psd::Written), ExportError> {
    let plan = psd::plan_export(d, &bake(), ctl)?;
    assert!(plan.blockers.is_empty(), "{:?}", plan.blockers);
    let mut out = Cursor::new(Vec::new());
    let written = plan.write_psd(d, ctl, &mut out, compression)?;
    Ok((out.into_inner(), written))
}

fn budget(mib: u64) -> ExportControl<'static> {
    ExportControl {
        source_budget: Some(mib * MIB),
        ..ExportControl::default()
    }
}

/// 書いた PSD の、レイヤーごと・チャンネルごとの (ID, 長さ, 圧縮の印) とレイヤーの矩形の高さ。
struct Chan {
    id: i16,
    len: u32,
    marker: u16,
    /// RLE のときの、行の長さの表。
    rows: Vec<u16>,
}
fn channels(bytes: &[u8]) -> Vec<Vec<Chan>> {
    let u16_at = |p: usize| u16::from_be_bytes(bytes[p..p + 2].try_into().unwrap());
    let u32_at = |p: usize| u32::from_be_bytes(bytes[p..p + 4].try_into().unwrap());
    let mut p = 26;
    p += 4 + u32_at(p) as usize; // 色モードデータ
    p += 4 + u32_at(p) as usize; // 画像リソース
    p += 4; // レイヤーとマスクの情報の長さ
    p += 4; // レイヤー情報の長さ
    let count = (u16_at(p) as i16).unsigned_abs() as usize;
    p += 2;
    let mut records = Vec::new();
    for _ in 0..count {
        let top = u32_at(p) as i32;
        let bottom = u32_at(p + 8) as i32;
        let n = u16_at(p + 16) as usize;
        p += 18;
        let mut list = Vec::new();
        for _ in 0..n {
            list.push((u16_at(p) as i16, u32_at(p + 2)));
            p += 6;
        }
        p += 12; // 署名・合成キー・不透明度ほか
        p += 4 + u32_at(p) as usize; // 付加情報
        records.push((list, (bottom - top) as usize));
    }
    let mut out = Vec::new();
    for (list, height) in records {
        let mut chans = Vec::new();
        for (id, len) in list {
            let marker = u16_at(p);
            let rows = if marker == 1 {
                (0..height).map(|y| u16_at(p + 2 + y * 2)).collect()
            } else {
                Vec::new()
            };
            chans.push(Chan {
                id,
                len,
                marker,
                rows,
            });
            p += len as usize;
        }
        out.push(chans);
    }
    out
}

fn layer(name: &str, id: i32, w: u32, h: u32, pixel: impl Fn(u32, u32) -> [u8; 4]) -> Layer {
    let mut pixels = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            pixels.extend(pixel(x, y))
        }
    }
    Layer {
        id,
        name: name.into(),
        left: 3,
        top: 2,
        width: w,
        height: h,
        pixels_rgba: pixels,
        ..Layer::default()
    }
}

/// 圧縮に向くレイヤー・向かないレイヤー・全部透明・単色・マスクつき・グループ・調整・塗りつぶしを持つ PSD の文書（上から下の並び）。
fn mixed() -> Psd {
    let mut seed = 7;
    let random: Vec<[u8; 4]> = (0..300 * 40)
        .map(|_| [noise(&mut seed), noise(&mut seed), noise(&mut seed), 255])
        .collect();
    let mut noisy = layer("ノイズ", 1, 300, 40, |x, y| {
        random[(y * 300 + x) as usize]
    });
    noisy.mask = Some(Mask {
        left: 10,
        top: 4,
        width: 150,
        height: 20,
        default_color: 255,
        enabled: true,
        density: 255,
        pixels: (0..150 * 20).map(|i| (i % 251) as u8).collect(),
    });
    let mut flat_mask = layer("全面のマスク", 2, 20, 20, |_, _| [9, 8, 7, 255]);
    flat_mask.mask = Some(Mask {
        left: 0,
        top: 0,
        width: 1,
        height: 1,
        default_color: 0,
        enabled: true,
        density: 200,
        pixels: vec![0],
    });
    let group = Layer {
        id: 5,
        name: "グループ".into(),
        kind: LayerKind::Group {
            children: vec![layer("中", 6, 140, 9, |x, _| [(x * 2) as u8, 50, 90, 255])],
            divider_id: 7,
        },
        ..Layer::default()
    };
    Psd {
        width: 323,
        height: 66,
        layers: vec![
            noisy,
            layer("グラデーション", 3, 256, 30, |x, y| {
                [x as u8, y as u8, 0, 128]
            }),
            layer("透明", 4, 200, 12, |_, _| [0, 0, 0, 0]),
            flat_mask,
            group,
            Layer {
                id: 8,
                name: "調整".into(),
                kind: LayerKind::Adjustment(psd::Adjustment::Invert),
                ..Layer::default()
            },
            Layer {
                id: 9,
                name: "単色".into(),
                kind: LayerKind::SolidColor([10, 20, 30]),
                ..Layer::default()
            },
            layer("地", 10, 320, 64, |x, y| {
                [(x / 4) as u8, (y * 3) as u8, 200, 255]
            }),
        ],
        composite_rgba: None,
    }
}

// ───────── RLE ─────────

#[test]
fn an_rle_psd_reads_back_the_same_through_every_reader_and_is_smaller() {
    let doc = mixed();
    let raw = psd::write(&doc, &Limits::default()).unwrap();
    let rle = psd::write_with(&doc, &Limits::default(), Compression::Rle).unwrap();
    assert!(rle.len() < raw.len(), "{} < {}", rle.len(), raw.len());
    // 厳密な読み（原本を保つ読み）: 同じ文書・同じ合成
    let a = psd::read(&raw, &Limits::default()).unwrap();
    let b = psd::read(&rle, &Limits::default()).unwrap();
    assert_eq!(
        a.mode(),
        psd::CompatibilityMode::EditableRaster,
        "{:?}",
        a.diagnostics()
    );
    assert_eq!(
        b.mode(),
        psd::CompatibilityMode::EditableRaster,
        "{:?}",
        b.diagnostics()
    );
    assert_eq!(a.document(), b.document());
    assert!(
        b.diagnostics().iter().all(|d| d.is_informational()),
        "{:?}",
        b.diagnostics()
    );
    // 写しとしての取り込み: 同じ合成
    let import = |bytes: &[u8]| match psd::import_copy(
        &mut Cursor::new(bytes),
        &psd::CopyOptions {
            source_budget: 256 * MIB,
            cancel: None,
        },
    )
    .unwrap()
    {
        psd::CopyOutcome::Imported(i) => i,
        psd::CopyOutcome::Refused(why) => panic!("{}", why.message()),
    };
    let (ia, ib) = (import(&raw), import(&rle));
    assert!(
        ib.notes
            .iter()
            .all(|n| n.action == psd::ImportAction::Ignored),
        "{:?}",
        ib.notes
    );
    assert_eq!(
        ia.document
            .composite_channel(Channel::Color, ia.document.bounds())
            .unwrap(),
        ib.document
            .composite_channel(Channel::Color, ib.document.bounds())
            .unwrap()
    );
    // 流して確かめる読み
    let verified = psd::verify_stream(&mut Cursor::new(&rle), None).unwrap();
    assert_eq!(verified.layers, 9, "グループの区切りを除くレイヤーの数");
    assert_eq!(verified.bytes, rle.len() as u64);
    assert_eq!(
        psd::verify_stream(&mut Cursor::new(&raw), None)
            .unwrap()
            .layers,
        9
    );
}

#[test]
fn each_channel_picks_rle_or_raw_and_the_row_table_adds_up() {
    let rle = psd::write_with(&mixed(), &Limits::default(), Compression::Rle).unwrap();
    let records = channels(&rle);
    // 記録は下から上: 地・（グループの区切り）・中・グループ・単色・調整・全面のマスク・透明・グラデーション・ノイズ
    let by_name = |i: usize| &records[i];
    let noisy = by_name(records.len() - 1);
    // 乱数の RGB は圧縮して大きくなるので無圧縮のまま、不透明の A は全部 255 なので RLE
    let markers: Vec<(i16, u16)> = noisy.iter().map(|c| (c.id, c.marker)).collect();
    assert_eq!(
        markers,
        [(0, 0), (1, 0), (2, 0), (-1, 1), (-2, 0)],
        "ノイズ: R・G・B は無圧縮、A は RLE、マスクは周期のあるだけの値で無圧縮"
    );
    for chans in &records {
        for c in chans {
            match c.marker {
                1 => {
                    let table: u32 = c.rows.iter().map(|n| u32::from(*n)).sum();
                    assert_eq!(
                        c.len,
                        2 + 2 * c.rows.len() as u32 + table,
                        "RLE の長さ = 印 + 表 + 行の合計（チャンネル {}）",
                        c.id
                    );
                    assert!(c.rows.iter().all(|n| *n > 0));
                }
                0 => {}
                other => panic!("圧縮の印 {other}"),
            }
        }
    }
    // 全部透明のレイヤー・単色は RLE で小さい（4 チャンネルすべて）
    let transparent = &records[records.len() - 3];
    assert!(
        transparent
            .iter()
            .take(4)
            .all(|c| c.marker == 1 && c.len < 200 * 12 / 4),
        "{:?}",
        transparent.iter().map(|c| c.len).collect::<Vec<_>>()
    );
    // グループ・調整・塗りつぶしの記録は、画素の無い印だけ（無圧縮の 2 バイト）
    let flat: Vec<&Chan> = records
        .iter()
        .filter(|c| c.iter().all(|c| c.len == 2))
        .flatten()
        .collect();
    assert!(!flat.is_empty() && flat.iter().all(|c| c.marker == 0));
}

#[test]
fn the_merged_image_is_rle_only_when_it_gets_smaller() {
    // 単色の統合画像は RLE（印は 1）、乱数の統合画像は無圧縮（印は 0）
    let merged_marker = |doc: &Psd| {
        let bytes = psd::write_with(doc, &Limits::default(), Compression::Rle).unwrap();
        let mut p = 26;
        let u32_at = |p: usize| u32::from_be_bytes(bytes[p..p + 4].try_into().unwrap());
        p += 4 + u32_at(p) as usize;
        p += 4 + u32_at(p) as usize;
        p += 4 + u32_at(p) as usize; // レイヤーとマスクの情報
        u16::from_be_bytes(bytes[p..p + 2].try_into().unwrap())
    };
    let solid = Psd {
        width: 67,
        height: 66,
        layers: vec![layer("単色", 1, 64, 64, |_, _| [1, 2, 3, 255])],
        composite_rgba: None,
    };
    assert_eq!(merged_marker(&solid), 1);
    let mut seed = 3;
    let random: Vec<[u8; 4]> = (0..64 * 64)
        .map(|_| {
            [
                noise(&mut seed),
                noise(&mut seed),
                noise(&mut seed),
                noise(&mut seed) | 1,
            ]
        })
        .collect();
    let noisy = Psd {
        width: 67,
        height: 66,
        layers: vec![layer("ノイズ", 1, 64, 64, |x, y| {
            random[(y * 64 + x) as usize]
        })],
        composite_rgba: None,
    };
    assert_eq!(merged_marker(&noisy), 0);
    for doc in [&solid, &noisy] {
        let bytes = psd::write_with(doc, &Limits::default(), Compression::Rle).unwrap();
        psd::verify_stream(&mut Cursor::new(bytes), None).unwrap();
    }
}

#[test]
fn rows_wider_than_a_run_and_odd_widths_round_trip() {
    // 幅 1・2・127・128・129・255・1000 のレイヤー（繰り返しと並びの境目）
    for width in [1u32, 2, 3, 127, 128, 129, 255, 1000] {
        let doc = Psd {
            width: width + 3,
            height: 6,
            layers: vec![layer("レイヤー", 1, width, 4, |x, y| {
                let v = if (x / 3 + y) % 2 == 0 {
                    40
                } else {
                    (x % 251) as u8
                };
                [v, v / 2, 255 - v, if x % 7 == 0 { 0 } else { 255 }]
            })],
            composite_rgba: None,
        };
        let raw = psd::write(&doc, &Limits::default()).unwrap();
        let rle = psd::write_with(&doc, &Limits::default(), Compression::Rle).unwrap();
        let (a, b) = (
            psd::read(&raw, &Limits::default()).unwrap(),
            psd::read(&rle, &Limits::default()).unwrap(),
        );
        assert_eq!(a.document(), b.document(), "幅 {width}");
        psd::verify_stream(&mut Cursor::new(&rle), None).unwrap();
    }
}

// ───────── 流して書く ─────────

/// 保存した画素は、タイルごとに行を写して書く。タイルの大きさで割り切れないキャンバス・まばらなタイル・透明の画素の RGB も、1 画素ずつ引いたものと同じ。
#[test]
fn a_stored_layer_is_copied_tile_by_tile_exactly_as_pixel_by_pixel() {
    let (w, h) = (50u32, 37u32);
    let mut d = Document::with_tile_size(w, h, 16).unwrap();
    let id = d.add_layer("まばら").unwrap();
    let scattered = [(3, 2), (20, 5), (49, 36), (33, 20), (17, 31), (48, 0)];
    for (n, (x, y)) in scattered.into_iter().enumerate() {
        // 3 つ目は透明で RGB を持つ（透明画素の RGB を守る）
        let a = if n == 2 { 0 } else { 255 - n as u8 };
        d.set_channel_pixel(
            id,
            Channel::Color,
            x,
            y,
            Rgba8::new(10 + n as u8, 20, 30 + n as u8, a),
        )
        .unwrap();
    }
    let out = psd::export_core(&d, &bake(), &ExportControl::default()).unwrap();
    let psd_layer = &out.document.layers[0];
    assert_eq!(psd_layer.name, "まばら");
    let surface = d.layer(id).unwrap().surface(Channel::Color).unwrap();
    // 矩形は、面のあるタイルの外接矩形（キャンバスの端で切る）。レイヤーの行は上から
    let (left, top, width, height) = (
        psd_layer.left as u32,
        psd_layer.top as u32,
        psd_layer.width,
        psd_layer.height,
    );
    assert!(
        left == 0 && top == 0 && width == w && height == h,
        "{left} {top} {width} {height}"
    );
    for y in 0..height {
        for x in 0..width {
            let want = surface
                .pixel(left + x, h - 1 - (top + y))
                .unwrap()
                .to_array();
            let at = ((y * width + x) * 4) as usize;
            assert_eq!(psd_layer.pixels_rgba[at..at + 4], want, "({x}, {y})");
        }
    }
    // 1 つのタイルだけのとき: 矩形はそのタイル（キャンバスの端で切れる）
    let mut e = Document::with_tile_size(w, h, 16).unwrap();
    let one = e.add_layer("端").unwrap();
    e.set_channel_pixel(one, Channel::Color, 49, 36, Rgba8::new(1, 2, 3, 255))
        .unwrap();
    let out = psd::export_core(&e, &bake(), &ExportControl::default()).unwrap();
    let l = &out.document.layers[0];
    assert_eq!(
        (l.left, l.top, l.width, l.height),
        (48, 0, 2, 5),
        "x 48..50・y は上から 0..5"
    );
    assert_eq!(l.pixels_rgba[4..8], [1, 2, 3, 255], "最上段（49, 36）");
}

fn busy_document() -> Document {
    let mut d = Document::with_tile_size(96, 64, 16).unwrap();
    let base = d.add_layer("下").unwrap();
    paint(&mut d, base, 96, 64, |x, y| {
        [(x * 2) as u8, (y * 3) as u8, 90, 255]
    });
    let soft = d.add_layer("ぼかし").unwrap();
    paint(&mut d, soft, 60, 40, |x, y| {
        [
            200,
            (x * 4) as u8,
            (y * 5) as u8,
            if x < 10 { 0 } else { 180 },
        ]
    });
    d.add_filter(
        soft,
        FilterTarget::Content,
        FilterSpec::new(EffectSettings::blur(3)),
    )
    .unwrap();
    let masked = d.add_layer("マスク").unwrap();
    paint(&mut d, masked, 50, 50, |x, _| [20, 120, (x * 5) as u8, 255]);
    d.add_layer_mask(masked).unwrap();
    d.add_fill_layer(
        "半透明",
        &[(Channel::Color, Rgba8::new(10, 20, 30, 100))],
        None,
    )
    .unwrap();
    d.add_fill_layer("単色", &[(Channel::Color, Rgba8::new(1, 2, 3, 255))], None)
        .unwrap();
    // 反転して、フィルターを通したマスク（マスクの画素に焼く）
    let inverted = d.add_layer("反転マスク").unwrap();
    paint(&mut d, inverted, 64, 48, |x, y| {
        [x as u8 * 3, 90, y as u8 * 4, 255]
    });
    d.add_layer_mask(inverted).unwrap();
    d.set_layer_mask_inverted(inverted, true).unwrap();
    d.add_filter(
        inverted,
        FilterTarget::Mask,
        FilterSpec::new(EffectSettings::invert()),
    )
    .unwrap();
    // マスクのあるグループ・クリッピングされたグループ（1 枚の画素のレイヤーに焼く）
    let inner = d.add_layer("中身").unwrap();
    paint(&mut d, inner, 40, 40, |x, _| [200, x as u8 * 6, 40, 220]);
    let clipped = d.group_layers(&[inner], "クリップ群").unwrap();
    d.set_layer_clipping(clipped, true).unwrap();
    let group = d.add_group("グループ", None).unwrap();
    d.add_layer_mask(group).unwrap();
    d.add_adjustment_layer("調整", AdjustmentSettings::invert(), None, None)
        .unwrap();
    d
}

#[test]
fn streaming_writes_the_same_bytes_as_building_in_memory_and_writing() {
    let d = busy_document();
    let ctl = ExportControl::default();
    for compression in [Compression::Raw, Compression::Rle] {
        let plan = psd::plan_export(&d, &bake(), &ctl).unwrap();
        assert!(!plan.notes.is_empty(), "焼くものがある");
        let built = plan.build(&d, &ctl).unwrap();
        let in_memory = psd::write_with(&built.document, &Limits::default(), compression).unwrap();
        let (streamed, written) = stream(&d, &ctl, compression).unwrap();
        assert_eq!(streamed, in_memory, "{compression:?}");
        assert_eq!(written.bytes, streamed.len() as u64);
        // クリッピングされたグループは 1 枚のレイヤーになり、中身のレイヤーは PSD に書かれない
        assert_eq!(written.layers, d.layers().len() - 1);
        let verified = psd::verify_stream(&mut Cursor::new(&streamed), None).unwrap();
        assert_eq!(verified.layers, written.layers);
    }
}

#[test]
fn a_streamed_psd_imports_with_the_composite_of_the_document() {
    let d = busy_document();
    let (bytes, _) = stream(&d, &budget(256), Compression::Rle).unwrap();
    let imported = match psd::import_copy(
        &mut Cursor::new(&bytes),
        &psd::CopyOptions {
            source_budget: 256 * MIB,
            cancel: None,
        },
    )
    .unwrap()
    {
        psd::CopyOutcome::Imported(i) => i,
        psd::CopyOutcome::Refused(why) => panic!("{}", why.message()),
    };
    assert_eq!(
        imported.document.layers().len(),
        d.layers().len() - 1,
        "クリッピングされたグループは 1 枚のレイヤーになる"
    );
    let want = d.composite_channel(Channel::Color, d.bounds()).unwrap();
    let got = imported
        .document
        .composite_channel(Channel::Color, imported.document.bounds())
        .unwrap();
    assert_eq!(want, got);
}

// ───────── 予算と上限 ─────────

#[test]
fn the_export_limits_follow_the_budget_like_the_import() {
    let l = Limits::for_export(256 * MIB);
    assert_eq!((l.max_layers, l.max_dimension), (256, 30000));
    assert_eq!(
        l.max_canvas_pixels,
        64 * MIB,
        "予算 256 MiB は 8192² の画素（4 バイト）"
    );
    assert_eq!(
        l.max_output_bytes,
        i32::MAX as usize,
        "ファイルは PSD の 2 GiB"
    );
    assert_eq!(
        Limits::for_export(8192 * MIB).max_layers,
        8192,
        "レイヤーの記録は予算 1 MiB につき 1 件"
    );
    assert_eq!(
        Limits::for_export(64 * 1024 * MIB).max_layers,
        32767,
        "PSD の上限"
    );
    assert_eq!(
        Limits::for_export(MIB).max_layers,
        256,
        "256 件は下回らない"
    );
}

#[test]
fn a_canvas_over_the_budget_is_refused_with_the_size_and_a_hint_it_can_be_raised() {
    let mut d = Document::new(2048, 2048).unwrap();
    d.add_layer("レイヤー").unwrap();
    // 2048² の画素は 16 MiB。予算 8 MiB には入らない
    let err = psd::check_exportable(&d, Channel::Color, &budget(8)).unwrap_err();
    let ExportError::Overrun(over) = err else {
        panic!("{err:?}")
    };
    assert_eq!(
        over,
        Overrun::Canvas {
            width: 2048,
            height: 2048
        }
    );
    assert!(
        over.raised_by_budget()
            && over.message().contains("2048×2048")
            && over.message().contains("予算")
    );
    assert!(psd::check_exportable(&d, Channel::Color, &budget(16)).is_ok());
    // 計画も構築も同じ理由で、Error::Budget として断る
    let err = psd::plan_export(&d, &bake(), &budget(8)).unwrap_err();
    assert!(matches!(err, yolu_io::Error::Budget(_)), "{err}");
    // 予算を決めない呼び方は、従来の固定の上限（4096²）
    let mut big = Document::new(5000, 5000).unwrap();
    big.add_layer("レイヤー").unwrap();
    assert!(psd::check_exportable(&big, Channel::Color, &ExportControl::default()).is_err());
    assert!(psd::check_exportable(&big, Channel::Color, &budget(256)).is_ok());
}

#[test]
fn layer_records_over_the_budget_count_group_dividers_and_name_the_numbers() {
    // 予算 64 MiB のレイヤーの記録は 256 件まで。レイヤー 300 枚は、文書の確かめで断る
    let mut d = Document::with_tile_size(16, 16, 16).unwrap();
    for n in 0..300 {
        d.add_layer(&format!("レイヤー{n}")).unwrap();
    }
    let err = psd::check_exportable(&d, Channel::Color, &budget(64)).unwrap_err();
    let ExportError::Overrun(over) = err else {
        panic!("{err:?}")
    };
    assert_eq!(
        over,
        Overrun::Layers {
            count: 300,
            limit: 256
        }
    );
    // レイヤー 130 枚はグループの区切りも数えて 260 件になる。書く前のレイヤーの構造で断る
    let mut d = Document::with_tile_size(16, 16, 16).unwrap();
    for n in 0..130 {
        d.add_group(&format!("グループ{n}"), None).unwrap();
    }
    assert!(psd::check_exportable(&d, Channel::Color, &budget(64)).is_ok());
    let err = stream(&d, &budget(64), Compression::Rle).unwrap_err();
    let ExportError::Overrun(over) = err else {
        panic!("{err:?}")
    };
    assert_eq!(
        over,
        Overrun::Layers {
            count: 260,
            limit: 256
        }
    );
    // 予算を上げれば書ける
    let (bytes, written) = stream(&d, &budget(512), Compression::Rle).unwrap();
    assert_eq!(written.layers, 130);
    psd::verify_stream(&mut Cursor::new(bytes), None).unwrap();
}

/// 4096² 級でなくても、レイヤーの画素の合計が予算を超える文書（焼くレイヤー 6 枚 × 16 MiB = 96 MiB を、予算 64 MiB で）。
fn over_the_total() -> Document {
    let mut d = Document::new(2048, 2048).unwrap();
    for n in 0..6 {
        d.add_fill_layer(
            &format!("ガラス{n}"),
            &[(Channel::Color, Rgba8::new(10 + n as u8, 20, 30, 100))],
            None,
        )
        .unwrap();
    }
    d
}

#[test]
fn streaming_has_no_cap_on_the_total_of_the_layers_but_building_in_memory_stops_at_the_budget() {
    let d = over_the_total();
    let ctl = budget(64);
    let plan = psd::plan_export(&d, &bake(), &ctl).unwrap();
    assert_eq!(plan.notes.len(), 6);
    // メモリに組む道は、全レイヤーの合計を予算で止める（レイヤーの名前つき）
    let err = plan.build(&d, &ctl).unwrap_err();
    assert!(
        matches!(err, yolu_io::Error::Budget(_))
            && err.to_string().contains("予算")
            && err.to_string().contains("ガラス"),
        "{err}"
    );
    // 流す道は、レイヤー 1 枚ぶんしか持たないので書ける。画素は圧縮されて小さい
    let (bytes, written) = stream(&d, &ctl, Compression::Rle).unwrap();
    assert_eq!(written.layers, 6);
    assert!(bytes.len() < 2 * MIB as usize, "{}", bytes.len());
    let verified = psd::verify_stream(&mut Cursor::new(&bytes), None).unwrap();
    assert_eq!(verified.layers, 6);
    // 無圧縮なら 6 枚 + 統合画像で 100 MiB を超える（圧縮が効いている）
    let raw = stream(&d, &ctl, Compression::Raw).unwrap().0;
    assert!(
        raw.len() as u64 > 100 * MIB && raw.len() > bytes.len() * 50,
        "{} / {}",
        raw.len(),
        bytes.len()
    );
}

#[test]
fn the_flat_export_and_a_normal_bake_build_in_memory_and_stop_at_the_budget_by_layer_name() {
    // 平らの 1 枚（全レイヤーをメモリに組む道）。キャンバスが予算に入れば書ける
    let mut d = Document::new(512, 512).unwrap();
    d.add_layer("レイヤー").unwrap();
    let flat = ExportOptions::new(Channel::Color, ExportMode::Flat);
    let ctl = budget(2);
    let plan = psd::plan_export(&d, &flat, &ctl).unwrap();
    let mut out = Cursor::new(Vec::new());
    plan.write_psd(&d, &ctl, &mut out, Compression::Rle)
        .unwrap();
    psd::verify_stream(&mut Cursor::new(out.into_inner()), None).unwrap();
    // Normal の焼き込みは、統合画像を書いたレイヤーから作るので全レイヤーをメモリに組む。レイヤー（512² = 1 MiB）の合計が予算 2 MiB を超える 3 枚目で、レイヤーの名前で断る
    let mut n = Document::new(512, 512).unwrap();
    for i in 0..3 {
        let l = n.add_layer(&format!("法線{i}")).unwrap();
        paint_channel(&mut n, l, Channel::Normal, 512, 512, |x, y| {
            [(x / 3) as u8, (y / 3) as u8, 255, 255]
        });
    }
    let normal = ExportOptions::new(Channel::Normal, ExportMode::Bake);
    let plan = psd::plan_export(&n, &normal, &ctl).unwrap();
    let mut out = Cursor::new(Vec::new());
    let err = plan
        .write_psd(&n, &ctl, &mut out, Compression::Rle)
        .unwrap_err();
    let ExportError::Overrun(over) = err else {
        panic!("{err:?}")
    };
    assert_eq!(
        over,
        Overrun::Memory {
            layer: "法線2".into()
        }
    );
    assert!(
        over.raised_by_budget()
            && over.message().contains("法線2")
            && over.message().contains("予算")
    );
    // 予算を上げれば書ける
    let ctl = budget(8);
    let plan = psd::plan_export(&n, &normal, &ctl).unwrap();
    let mut out = Cursor::new(Vec::new());
    let written = plan
        .write_psd(&n, &ctl, &mut out, Compression::Rle)
        .unwrap();
    assert_eq!(written.layers, 3);
    psd::verify_stream(&mut Cursor::new(out.into_inner()), None).unwrap();
}

#[test]
fn a_file_over_the_size_limit_is_refused_naming_the_layer_that_crosses_it() {
    // 乱数のレイヤーは圧縮できない。ファイルの上限を 300 KB にすると、書いている途中のレイヤーの名前で断る
    let mut d = Document::with_tile_size(128, 128, 16).unwrap();
    let mut seed = 1;
    for n in 0..4 {
        let l = d.add_layer(&format!("乱数{n}")).unwrap();
        let mut rows = vec![[0u8; 4]; 128 * 128];
        for p in &mut rows {
            *p = [noise(&mut seed), noise(&mut seed), noise(&mut seed), 255];
        }
        paint(&mut d, l, 128, 128, |x, y| rows[(y * 128 + x) as usize]);
    }
    let ctl = ExportControl {
        source_budget: Some(256 * MIB),
        max_file_bytes: Some(160 * 1024),
        ..ExportControl::default()
    };
    let err = stream(&d, &ctl, Compression::Rle).unwrap_err();
    let ExportError::Overrun(over) = err else {
        panic!("{err:?}")
    };
    // 1 枚 = RGB が無圧縮 48 KB + A が RLE。記録のレイヤーは下から（乱数0 が先）、書いた合計が 160 KB を超えるのは 4 枚目より前
    match &over {
        Overrun::FileSize { layer } => assert!(layer.starts_with("乱数"), "{layer}"),
        other => panic!("{other:?}"),
    }
    assert!(
        !over.raised_by_budget(),
        "PSD の 2 GiB は予算を上げても書けない"
    );
    assert!(over.message().contains("2 GiB"), "{}", over.message());
    // 上限を上げれば書ける。上限は PSD の 2 GiB を超えて指定できない
    let ctl = ExportControl {
        max_file_bytes: Some(u64::MAX),
        ..budget(256)
    };
    assert!(stream(&d, &ctl, Compression::Rle).is_ok());
}

#[test]
fn a_cancel_flag_stops_the_streaming_write() {
    let d = busy_document();
    let stop = AtomicBool::new(true);
    let ctl = ExportControl {
        cancel: Some(&stop),
        ..budget(256)
    };
    let plan = psd::plan_export(&d, &bake(), &ExportControl::default()).unwrap();
    let mut out = Cursor::new(Vec::new());
    let err = plan
        .write_psd(&d, &ctl, &mut out, Compression::Rle)
        .unwrap_err();
    assert!(
        matches!(
            &err,
            ExportError::Other(yolu_io::Error::Core(yolu_core::CoreError::Cancelled))
        ),
        "{err:?}"
    );
}

/// 書き込みの回ごとに「ここで取消の旗を立てる」かを決める出力（書いている途中の取消を決まった所で起こす）。旗が立ってから書かれたバイト数も数える
/// （取消の確かめが、旗が立ったあとに何も書かないことを固定する）。
struct Trip<'a, F: FnMut(usize, usize) -> bool> {
    inner: Cursor<Vec<u8>>,
    flag: &'a AtomicBool,
    /// (何回目の書き込みか（1 から）, その長さ) で、旗を立てる回なら true。
    trip: F,
    calls: usize,
    after_trip: u64,
}
impl<'a, F: FnMut(usize, usize) -> bool> Trip<'a, F> {
    fn new(flag: &'a AtomicBool, trip: F) -> Self {
        Self {
            inner: Cursor::new(Vec::new()),
            flag,
            trip,
            calls: 0,
            after_trip: 0,
        }
    }
}
impl<F: FnMut(usize, usize) -> bool> std::io::Write for Trip<'_, F> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        use std::sync::atomic::Ordering;
        self.calls += 1;
        if self.flag.load(Ordering::Relaxed) {
            self.after_trip += buf.len() as u64;
        }
        let n = self.inner.write(buf)?;
        if (self.trip)(self.calls, buf.len()) {
            self.flag.store(true, Ordering::Relaxed);
        }
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
impl<F: FnMut(usize, usize) -> bool> std::io::Seek for Trip<'_, F> {
    fn seek(&mut self, to: std::io::SeekFrom) -> std::io::Result<u64> {
        self.inner.seek(to)
    }
}

/// 無圧縮で書く、`h` 行のラスターレイヤー 1 枚（行は 64 バイト）の文書。
fn tall() -> Document {
    let mut d = Document::with_tile_size(64, 300, 16).unwrap();
    let l = d.add_layer("縦長").unwrap();
    paint(&mut d, l, 64, 300, |x, y| [x as u8, y as u8, 7, 255]);
    d
}

/// 取消が、流して書く途中の 3 か所（記録ごと・チャンネルの 128 行ごと・統合画像の前）で効く。旗が立ったあとに書くバイト数で、どの確かめが止めたかを固定する。
#[test]
fn a_cancel_raised_while_streaming_stops_at_the_next_check_and_writes_nothing_more() {
    use std::sync::atomic::Ordering;
    let cancelled = |err: &ExportError| {
        matches!(
            err,
            ExportError::Other(yolu_io::Error::Core(yolu_core::CoreError::Cancelled))
        )
    };
    // 記録ごと: 先頭の記録がグループの区切り（画素も無い）なので、ここで止めなければ区切りの 8 バイトが書かれる
    let mut g = Document::with_tile_size(32, 32, 16).unwrap();
    let l = g.add_layer("葉").unwrap();
    paint(&mut g, l, 32, 32, |x, y| [x as u8, y as u8, 1, 255]);
    g.group_layers(&[l], "枝").unwrap();
    let stop = AtomicBool::new(false);
    let ctl = ExportControl {
        cancel: Some(&stop),
        ..budget(256)
    };
    let plan = psd::plan_export(&g, &bake(), &ctl).unwrap();
    let mut out = Trip::new(&stop, |call, _| call == 1);
    let err = plan
        .write_psd(&g, &ctl, &mut out, Compression::Raw)
        .unwrap_err();
    assert!(cancelled(&err), "{err:?}");
    assert_eq!(out.after_trip, 0, "記録の先頭の確かめで止まる");
    // チャンネルの 128 行ごと: 1 行目のあとで旗が立つ。次の確かめは 128 行目で、その手前の 127 行（64 バイト）は書かれる。確かめが無ければレイヤーの終わりまで書く
    let d = tall();
    stop.store(false, Ordering::Relaxed);
    let ctl = ExportControl {
        cancel: Some(&stop),
        ..budget(256)
    };
    let plan = psd::plan_export(&d, &bake(), &ctl).unwrap();
    // 書き込みは、先頭（1）・チャンネルの印（2）・1 行目（3）
    let mut out = Trip::new(&stop, |call, _| call == 3);
    let err = plan
        .write_psd(&d, &ctl, &mut out, Compression::Raw)
        .unwrap_err();
    assert!(cancelled(&err), "{err:?}");
    assert_eq!(out.after_trip, 127 * 64, "128 行ごとの確かめで止まる");
    // 統合画像の前: レイヤーを書き終えた最後の 4 バイト（全体のレイヤーマスク情報）のあとで旗が立つ。確かめが無ければ統合画像の印が書かれる
    stop.store(false, Ordering::Relaxed);
    let mut out = Trip::new(&stop, |_, len| len == 4);
    let err = plan
        .write_psd(&d, &ctl, &mut out, Compression::Raw)
        .unwrap_err();
    assert!(cancelled(&err), "{err:?}");
    assert_eq!(out.after_trip, 0, "統合画像の前の確かめで止まる");
    // 旗が立たなければ書き終える
    stop.store(false, Ordering::Relaxed);
    let mut out = Trip::new(&stop, |_, _| false);
    plan.write_psd(&d, &ctl, &mut out, Compression::Raw)
        .unwrap();
    psd::verify_stream(&mut Cursor::new(out.inner.into_inner()), None).unwrap();
}

/// 全レイヤーをメモリに組む道（Normal の焼き込み・平らの 1 枚）も、呼び手が小さくしたファイルの上限で断る（流す道と同じ）。
#[test]
fn the_memory_path_honors_the_callers_file_limit_too() {
    let limited = |bytes| ExportControl {
        max_file_bytes: Some(bytes),
        ..budget(8)
    };
    // Normal の焼き込み
    let mut n = Document::new(512, 512).unwrap();
    for i in 0..3 {
        let l = n.add_layer(&format!("法線{i}")).unwrap();
        paint_channel(&mut n, l, Channel::Normal, 512, 512, |x, y| {
            [(x / 3) as u8, (y / 3) as u8, 255, 255]
        });
    }
    // 平らの 1 枚
    let mut f = Document::new(512, 512).unwrap();
    f.add_layer("レイヤー").unwrap();
    for (name, doc, options) in [
        (
            "Normal",
            &n,
            ExportOptions::new(Channel::Normal, ExportMode::Bake),
        ),
        (
            "平ら",
            &f,
            ExportOptions::new(Channel::Color, ExportMode::Flat),
        ),
    ] {
        let ctl = limited(8 * 1024);
        let plan = psd::plan_export(doc, &options, &ctl).unwrap();
        let mut out = Cursor::new(Vec::new());
        let err = plan
            .write_psd(doc, &ctl, &mut out, Compression::Rle)
            .unwrap_err();
        let ExportError::Overrun(over) = err else {
            panic!("{name}: {err:?}")
        };
        assert!(
            matches!(over, Overrun::FileSize { .. }) && !over.raised_by_budget(),
            "{name}: {over:?}"
        );
        // 上限が足りれば、同じ文書が書ける
        let ctl = limited(i32::MAX as u64);
        let plan = psd::plan_export(doc, &options, &ctl).unwrap();
        let mut out = Cursor::new(Vec::new());
        plan.write_psd(doc, &ctl, &mut out, Compression::Rle)
            .unwrap();
        psd::verify_stream(&mut Cursor::new(out.into_inner()), None).unwrap();
    }
}

/// ファイルの上限は、圧縮したあとの大きさ（RLE）で見る。無圧縮で書くと超える文書でも、圧縮して収まれば書ける（無圧縮は従来どおり書く前に断る）。
#[test]
fn the_file_limit_counts_the_compressed_size_so_a_document_that_only_fits_compressed_is_written() {
    let doc = Psd {
        width: 210,
        height: 210,
        layers: vec![layer("単色", 1, 200, 200, |_, _| [10, 20, 30, 255])],
        composite_rgba: None,
    };
    // 無圧縮はレイヤーと統合画像で 300 KB 超。RLE は数 KB
    let limits = Limits {
        max_output_bytes: 20_000,
        ..Limits::default()
    };
    let err = psd::write_with(&doc, &limits, Compression::Raw).unwrap_err();
    assert!(matches!(err, yolu_io::Error::Budget(_)), "{err}");
    let rle = psd::write_with(&doc, &limits, Compression::Rle).unwrap();
    assert!(rle.len() < 20_000, "{}", rle.len());
    assert_eq!(
        psd::verify_stream(&mut Cursor::new(&rle), None)
            .unwrap()
            .layers,
        1
    );
    // 圧縮したあとの大きさが上限を超えれば、RLE でも断る
    let tight = Limits {
        max_output_bytes: rle.len() - 1,
        ..Limits::default()
    };
    let err = psd::write_with(&doc, &tight, Compression::Rle).unwrap_err();
    assert!(matches!(err, yolu_io::Error::Budget(_)), "{err}");
    let exact = Limits {
        max_output_bytes: rle.len(),
        ..Limits::default()
    };
    assert_eq!(
        psd::write_with(&doc, &exact, Compression::Rle).unwrap(),
        rle
    );
}

/// 統合画像を書いて上限を超えるときは、レイヤーの名前が空。画面の文は「統合画像」と言う（「レイヤー「」」と言わない）。
#[test]
fn a_file_over_the_limit_at_the_merged_image_says_so_instead_of_naming_an_empty_layer() {
    let d = busy_document();
    let (bytes, _) = stream(&d, &budget(256), Compression::Raw).unwrap();
    // レイヤーを書き終えても収まるが、統合画像まで書くと超える上限（1 バイト足りない）
    let ctl = ExportControl {
        max_file_bytes: Some(bytes.len() as u64 - 1),
        ..budget(256)
    };
    let err = stream(&d, &ctl, Compression::Raw).unwrap_err();
    let ExportError::Overrun(over) = err else {
        panic!("{err:?}")
    };
    assert_eq!(
        over,
        Overrun::FileSize {
            layer: String::new()
        }
    );
    let message = over.message();
    assert!(
        message.contains("統合画像") && message.contains("2 GiB") && !message.contains("「」"),
        "{message}"
    );
    // Error::Budget へ変換される道（write_with）でも同じ文
    let converted: yolu_io::Error = over.into();
    assert!(converted.to_string().contains("統合画像"), "{converted}");
    let ctl = ExportControl {
        max_file_bytes: Some(bytes.len() as u64),
        ..budget(256)
    };
    assert!(stream(&d, &ctl, Compression::Raw).is_ok());
}

/// 辺が形式の上限（30000）を超えるキャンバスは、画素の予算の断りと別の理由で、設定を上げても書けないと言う。
#[test]
fn a_side_over_the_format_limit_is_told_apart_from_a_canvas_over_the_budget() {
    // 32768 × 16 は画素は少ないが、PSD の辺の上限を超える。予算を上げても書けない
    let mut wide = Document::new(32768, 16).unwrap();
    wide.add_layer("レイヤー").unwrap();
    let err = psd::check_exportable(&wide, Channel::Color, &budget(8192)).unwrap_err();
    let ExportError::Overrun(over) = err else {
        panic!("{err:?}")
    };
    assert_eq!(
        over,
        Overrun::Side {
            width: 32768,
            height: 16,
            limit: 30000
        }
    );
    assert!(
        !over.raised_by_budget() && over.message().contains("30000"),
        "{}",
        over.message()
    );
    // 予算を決めない既定の上限（辺 8192）の断りは、予算を決めれば書ける
    let mut mid = Document::new(9000, 100).unwrap();
    mid.add_layer("レイヤー").unwrap();
    let err = psd::check_exportable(&mid, Channel::Color, &ExportControl::default()).unwrap_err();
    let ExportError::Overrun(over) = err else {
        panic!("{err:?}")
    };
    assert_eq!(
        over,
        Overrun::Side {
            width: 9000,
            height: 100,
            limit: 8192
        }
    );
    assert!(over.raised_by_budget());
    assert!(psd::check_exportable(&mid, Channel::Color, &budget(256)).is_ok());
    // 辺が収まって画素の数が予算を超えるのは、従来どおりのキャンバスの断り
    let mut big = Document::new(8000, 8000).unwrap();
    big.add_layer("レイヤー").unwrap();
    let err = psd::check_exportable(&big, Channel::Color, &budget(64)).unwrap_err();
    let ExportError::Overrun(over) = err else {
        panic!("{err:?}")
    };
    assert_eq!(
        over,
        Overrun::Canvas {
            width: 8000,
            height: 8000
        }
    );
    assert!(over.raised_by_budget());
}

/// 書いたバイト列の照合: 書いたとおりのファイルは合い、構造を壊さない画素の化け・先頭の化け・切れ・余りは見つける。小分けに読んでも同じ。取消で止まる。
#[test]
fn the_checksum_matches_what_was_written_and_finds_changes_the_structure_check_cannot() {
    struct Dribble<'a>(&'a [u8]);
    impl std::io::Read for Dribble<'_> {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let n = buf.len().min(self.0.len()).min(7);
            buf[..n].copy_from_slice(&self.0[..n]);
            self.0 = &self.0[n..];
            Ok(n)
        }
    }
    let d = busy_document();
    for compression in [Compression::Raw, Compression::Rle] {
        let (bytes, written) = stream(&d, &ExportControl::default(), compression).unwrap();
        let sum = written.checksum;
        let matches = |b: &[u8]| sum.matches(&mut Cursor::new(b), None).unwrap();
        assert!(matches(&bytes), "{compression:?}");
        assert!(
            sum.matches(&mut Dribble(&bytes), None).unwrap(),
            "{compression:?}: 小分けに読んでも同じ"
        );
        // 最後の 1 バイトは統合画像の最後の行の画素（制御の印ではない）。読み戻しの復号は通るが、書いたバイト列とは違う
        let mut flipped = bytes.clone();
        *flipped.last_mut().unwrap() ^= 1;
        assert!(
            psd::verify_stream(&mut Cursor::new(&flipped), None).is_ok(),
            "{compression:?}: 構造は壊れない"
        );
        assert!(!matches(&flipped), "{compression:?}: 画素の化け");
        // レイヤーの画素の途中の化け（先頭と最後のあいだ）
        let mut inner = bytes.clone();
        let at = bytes.len() * 3 / 4;
        inner[at] ^= 0x40;
        assert!(!matches(&inner), "{compression:?}: 途中の化け");
        // 先頭（ヘッダーとレイヤーの記録の表）の化け
        let mut head = bytes.clone();
        head[40] ^= 1;
        assert!(!matches(&head), "{compression:?}: 先頭の化け");
        // 切れ・余り
        assert!(!matches(&bytes[..bytes.len() - 1]) && !matches(&bytes[..100]) && !matches(&[]));
        let mut longer = bytes.clone();
        longer.push(0);
        assert!(!matches(&longer), "{compression:?}: 余り");
        // 取消
        let stop = AtomicBool::new(true);
        let err = sum
            .matches(&mut Cursor::new(&bytes), Some(&stop))
            .unwrap_err();
        assert!(
            matches!(err, yolu_io::Error::Core(yolu_core::CoreError::Cancelled)),
            "{err:?}"
        );
    }
}

/// 書き手が書いたとおりなら、読み戻しの確かめが落とす・変わるものとして断る PSD は無い。断るときの文は画面にそのまま出るので、内部の種類の名前を入れない。
#[test]
fn the_verifying_read_refusal_for_things_the_import_would_change_names_no_internal_kind() {
    let good = psd::write_with(&mixed(), &Limits::default(), Compression::Rle).unwrap();
    let u16_at = |b: &[u8], p: usize| u16::from_be_bytes(b[p..p + 2].try_into().unwrap()) as usize;
    let u32_at = |b: &[u8], p: usize| u32::from_be_bytes(b[p..p + 4].try_into().unwrap());
    // 最初のレイヤーの付加情報の終わりに、レイヤー効果（取り込みで落とす）の印を足す
    let mut p = 26;
    p += 4 + u32_at(&good, p) as usize;
    p += 4 + u32_at(&good, p) as usize;
    let (lm_at, info_at) = (p, p + 4);
    p += 8 + 2;
    let extra_at = p + 18 + u16_at(&good, p + 16) * 6 + 12;
    let insert_at = extra_at + 4 + u32_at(&good, extra_at) as usize;
    let mut tag = b"8BIMlfx2".to_vec();
    tag.extend(4u32.to_be_bytes());
    tag.extend([0; 4]);
    let mut patched = good[..insert_at].to_vec();
    patched.extend(&tag);
    patched.extend(&good[insert_at..]);
    for at in [extra_at, info_at, lm_at] {
        let v = u32_at(&patched, at) + tag.len() as u32;
        patched[at..at + 4].copy_from_slice(&v.to_be_bytes());
    }
    let err = psd::verify_stream(&mut Cursor::new(&patched), None).unwrap_err();
    let yolu_io::Error::InvalidData(message) = &err else {
        panic!("{err:?}")
    };
    assert!(message.contains("取り込みで落とす"), "{message}");
    assert!(
        !message.contains("LayerEffects")
            && !message
                .replace("PSD", "")
                .chars()
                .any(|c| c.is_ascii_alphabetic()),
        "{message}"
    );
}

/// レイヤーの下に `depth` 段のグループを入れ子にした文書。
fn nested(depth: usize) -> Document {
    let mut d = Document::with_tile_size(16, 16, 16).unwrap();
    let mut current = d.add_layer("葉").unwrap();
    for n in 0..depth {
        current = d.group_layers(&[current], &format!("段{n}")).unwrap();
    }
    d
}

/// PSD 側の型で `depth` 段のグループを手で組む（core を通さないので、core が作らせない深さも組める）。
fn psd_nested(depth: usize) -> Psd {
    let mut inner = layer("葉", 1, 4, 4, |_, _| [1, 2, 3, 255]);
    for n in 0..depth as i32 {
        inner = Layer {
            id: 2 + 2 * n,
            name: format!("段{n}"),
            kind: LayerKind::Group {
                children: vec![inner],
                divider_id: 3 + 2 * n,
            },
            ..Layer::default()
        };
    }
    Psd {
        width: 16,
        height: 16,
        layers: vec![inner],
        composite_rgba: None,
    }
}

#[test]
fn the_exports_group_nesting_limit_follows_the_budget_and_equals_the_documents_limit() {
    // 予算から決める書き出しの上限（設定の「レイヤーの画素」から。`for_export`）の入れ子は、文書の上限と同じ値（128 などへ戻すと落ちる）。
    // 文書の入れ子はその値までしか作れないので、ここは PSD 側の型で組み、上限の段は書けて、1 段多いものは入れ子の予算で断る
    let limit = yolu_core::MAX_GROUP_DEPTH;
    for budget in [64 * MIB, 512 * MIB, 4096 * MIB] {
        let limits = Limits::for_export(budget);
        assert_eq!(limits.max_group_depth, limit, "{budget}");
        psd::write_with(&psd_nested(limit), &limits, Compression::Rle).unwrap();
        let err = psd::write_with(&psd_nested(limit + 1), &limits, Compression::Rle).unwrap_err();
        assert!(
            matches!(err, yolu_io::Error::Budget(_)),
            "{budget}: {err:?}"
        );
    }
}

#[test]
fn nested_groups_stream_to_the_document_limit_without_overflowing_a_small_stack_and_no_further() {
    // 文書の入れ子は `MAX_GROUP_DEPTH` 段まで（core の編集と読み込みが守る。書き出しの上限も同じ値）。2 MiB のスタックのスレッドでも、
    // 再帰が溢れない。予算を渡さない書き出しは、C# の書き手と対の固定の上限（32 段）で断る
    std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(|| {
            let limit = yolu_core::MAX_GROUP_DEPTH;
            let d = nested(limit);
            let (bytes, written) = stream(&d, &budget(512), Compression::Rle).unwrap();
            assert_eq!(written.layers, limit + 1);
            assert_eq!(
                psd::verify_stream(&mut Cursor::new(&bytes), None)
                    .unwrap()
                    .layers,
                limit + 1
            );
            // 固定の上限を超える入れ子は、入れ子の予算で断る
            let fixed = Limits::default().max_group_depth;
            assert_eq!(fixed, 32);
            stream(&nested(fixed), &ExportControl::default(), Compression::Rle).unwrap();
            let err = stream(
                &nested(fixed + 1),
                &ExportControl::default(),
                Compression::Rle,
            )
            .unwrap_err();
            assert!(
                matches!(err, ExportError::Other(yolu_io::Error::Budget(_))),
                "{err:?}"
            );
        })
        .unwrap()
        .join()
        .unwrap();
}

/// 厳密な書き出し（`from_core`）の断り方は変わらず、使う上限の値だけ呼び手から決められる。
#[test]
fn the_strict_export_keeps_its_refusals_and_takes_only_the_limits_from_the_caller() {
    // 4096² のキャンバスに 9 枚のマスク: キャンバス 1 枚ぶん（16 MiB）を数えるので、既定の 128 MiB は超える。予算 256 MiB を渡せば書ける
    let mut d = Document::new(4096, 4096).unwrap();
    for n in 0..9 {
        let a = d.add_layer(&format!("m{n}")).unwrap();
        d.add_layer_mask(a).unwrap();
    }
    let err = Psd::from_core(&d).unwrap_err();
    assert!(
        matches!(err, yolu_io::Error::Budget(_)) && err.to_string().contains("予算"),
        "{err}"
    );
    let doc = Psd::from_core_with(&d, &budget(256)).unwrap();
    assert_eq!(doc.layers.len(), 9);
    // 何を断るかは同じ（焼かないと書けないものは、予算を渡してもレイヤーの名前つきで断る）
    let mut blur = Document::new(32, 32).unwrap();
    let layer = blur.add_layer("ぼかし").unwrap();
    blur.add_filter(
        layer,
        FilterTarget::Content,
        FilterSpec::new(EffectSettings::blur(3)),
    )
    .unwrap();
    for ctl in [ExportControl::default(), budget(256)] {
        let err = Psd::from_core_with(&blur, &ctl).unwrap_err().to_string();
        assert!(
            err.contains("ぼかし") && err.contains("フィルター"),
            "{err}"
        );
    }
}

// ───────── 確かめの読み ─────────

/// 書き手が書ける形（全部の合成モード・全種類の調整・ロック・マスクの有効/無効と濃度・単色・グループの通過）は、確かめの読みがどれも受け入れる
/// （取り込みで落とす・変わるものとして断らない）。圧縮の有無が変わっても同じ。
#[test]
fn the_verifying_read_accepts_everything_the_writer_can_write() {
    use psd::{Adjustment as A, BlendMode as B, GradientColorStop as C, GradientOpacityStop as O};
    let mut layers = Vec::new();
    let mut id = 1;
    let mut next = || {
        id += 1;
        id
    };
    for mode in B::ALL.into_iter().filter(|m| *m != B::PassThrough) {
        let mut l = layer(&format!("{mode:?}"), next(), 20, 6, |x, y| {
            [x as u8 * 9, y as u8 * 30, 7, 200]
        });
        l.blend_mode = mode;
        l.opacity = 100 + mode as u8;
        l.visible = !(mode as usize).is_multiple_of(5);
        l.locks = [0, 1, 2, 4, 0x8000_0000][mode as usize % 5];
        l.clipping = mode as usize % 7 == 3;
        layers.push(l);
    }
    let adjustments = [
        A::Invert,
        A::Levels {
            input_black: 20,
            input_white: 230,
            output_black: 10,
            output_white: 240,
            gamma: 137,
        },
        A::HueSaturation {
            hue: -73,
            saturation: 42,
            lightness: -18,
        },
        A::GradientMap {
            reverse: true,
            colors: vec![
                C {
                    location: 0,
                    midpoint: 50,
                    rgb: [0, 0, 0],
                },
                C {
                    location: 2048,
                    midpoint: 40,
                    rgb: [200, 30, 30],
                },
                C {
                    location: 4096,
                    midpoint: 50,
                    rgb: [255, 255, 255],
                },
            ],
            opacities: vec![
                O {
                    location: 0,
                    midpoint: 50,
                    opacity: 255,
                },
                O {
                    location: 4096,
                    midpoint: 50,
                    opacity: 128,
                },
            ],
        },
        A::ToneCurve {
            composite: vec![[0, 0], [100, 140], [255, 255]],
            red: vec![[0, 0], [255, 255]],
            green: vec![[0, 10], [255, 250]],
            blue: vec![[0, 0], [128, 100], [255, 255]],
        },
        A::ColorBalance {
            shadows: [10, -20, 30],
            midtones: [0, 0, 0],
            highlights: [-40, 20, 5],
            preserve_luminosity: true,
        },
        A::BrightnessContrast {
            brightness: 40,
            contrast: -20,
        },
        A::Threshold { level: 128 },
        A::Posterize { levels: 6 },
    ];
    for (n, a) in adjustments.into_iter().enumerate() {
        layers.push(Layer {
            id: next(),
            name: format!("調整{n}"),
            opacity: 200,
            kind: LayerKind::Adjustment(a),
            ..Layer::default()
        });
    }
    layers.push(Layer {
        id: next(),
        name: "単色".into(),
        kind: LayerKind::SolidColor([1, 2, 3]),
        ..Layer::default()
    });
    let mut masked = layer("マスク", next(), 20, 6, |_, _| [5, 5, 5, 255]);
    masked.mask = Some(Mask {
        left: 2,
        top: 1,
        width: 4,
        height: 3,
        default_color: 255,
        enabled: false,
        density: 90,
        pixels: vec![0, 255, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16],
    });
    layers.push(masked);
    layers.push(Layer {
        id: next(),
        name: "通過".into(),
        blend_mode: B::PassThrough,
        kind: LayerKind::Group {
            children: vec![layer("子", next(), 5, 5, |_, _| [9, 9, 9, 255])],
            divider_id: next(),
        },
        ..Layer::default()
    });
    layers.reverse();
    let doc = Psd {
        width: 40,
        height: 30,
        layers,
        composite_rgba: None,
    };
    for compression in [Compression::Raw, Compression::Rle] {
        let bytes = psd::write_with(&doc, &Limits::default(), compression).unwrap();
        let verified = psd::verify_stream(&mut Cursor::new(&bytes), None)
            .unwrap_or_else(|e| panic!("{compression:?}: {e}"));
        assert_eq!(verified.layers, 26 + 9 + 1 + 1 + 2, "{compression:?}");
    }
}

#[test]
fn the_verifying_read_refuses_truncated_and_damaged_files() {
    let good = psd::write_with(&mixed(), &Limits::default(), Compression::Rle).unwrap();
    assert!(psd::verify_stream(&mut Cursor::new(&good), None).is_ok());
    // 途中で切れた
    for cut in [10, 40, good.len() / 3, good.len() - 5] {
        let err = psd::verify_stream(&mut Cursor::new(&good[..cut]), None).unwrap_err();
        assert!(
            matches!(err, yolu_io::Error::InvalidData(_)),
            "{cut}: {err:?}"
        );
    }
    // 余りがある
    let mut longer = good.clone();
    longer.push(0);
    assert!(psd::verify_stream(&mut Cursor::new(&longer), None).is_err());
    // RLE の行の長さの表が壊れた（最初の RLE のチャンネルの表の 1 つ目を 1 増やす）
    let records = channels(&good);
    let mut p = {
        let u32_at = |p: usize| u32::from_be_bytes(good[p..p + 4].try_into().unwrap());
        let mut p = 26;
        p += 4 + u32_at(p) as usize;
        p += 4 + u32_at(p) as usize;
        p + 4 + 4 + 2
    };
    // 記録を読み飛ばしてデータの先頭へ
    let count = records.len();
    for _ in 0..count {
        let n = u16::from_be_bytes(good[p + 16..p + 18].try_into().unwrap()) as usize;
        p += 18 + n * 6 + 12;
        let extra = u32::from_be_bytes(good[p..p + 4].try_into().unwrap()) as usize;
        p += 4 + extra;
    }
    let mut at = p;
    'find: for chans in &records {
        for c in chans {
            if c.marker == 1 {
                at += 2;
                break 'find;
            }
            at += c.len as usize;
        }
    }
    let mut broken = good.clone();
    let row = u16::from_be_bytes(broken[at..at + 2].try_into().unwrap()) + 1;
    broken[at..at + 2].copy_from_slice(&row.to_be_bytes());
    assert!(psd::verify_stream(&mut Cursor::new(&broken), None).is_err());
    // PSD でない
    assert!(psd::verify_stream(
        &mut Cursor::new(b"not a psd, not even close".to_vec()),
        None
    )
    .is_err());
    // 取消
    let stop = AtomicBool::new(true);
    let err = psd::verify_stream(&mut Cursor::new(&good), Some(&stop)).unwrap_err();
    assert!(
        matches!(err, yolu_io::Error::Core(yolu_core::CoreError::Cancelled)),
        "{err:?}"
    );
}

// ───────── 実物の大きさの測定（`cargo test -p yolu-io --test psd_io psd_stream::measure -- --ignored --nocapture --test-threads=1`） ─────────

/// 書いたものを数えるだけで捨てる出力（無圧縮の大きさを、メモリもディスクも使わずに知る）。
struct Null {
    pos: u64,
    len: u64,
}
impl std::io::Write for Null {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.pos += buf.len() as u64;
        self.len = self.len.max(self.pos);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
impl std::io::Seek for Null {
    fn seek(&mut self, to: std::io::SeekFrom) -> std::io::Result<u64> {
        self.pos = match to {
            std::io::SeekFrom::Start(p) => p,
            std::io::SeekFrom::End(d) => (self.len as i64 + d) as u64,
            std::io::SeekFrom::Current(d) => (self.pos as i64 + d) as u64,
        };
        Ok(self.pos)
    }
}

fn proc_kib(key: &str) -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .unwrap_or_default()
        .lines()
        .find_map(|l| l.strip_prefix(key))
        .and_then(|v| v.split_whitespace().next()?.parse().ok())
        .unwrap_or(0)
}

/// 矩形（core の座標。下から）を `pixel(x, y)` で塗る。
fn paint_rect(
    d: &mut Document,
    layer: LayerId,
    (x0, y0, w, h): (u32, u32, u32, u32),
    mut pixel: impl FnMut(u32, u32) -> [u8; 4],
) {
    let ts = d.tile_size();
    for ty in y0 / ts..(y0 + h).div_ceil(ts) {
        for tx in x0 / ts..(x0 + w).div_ceil(ts) {
            let mut tile = vec![0u8; (ts * ts * 4) as usize];
            for y in 0..ts {
                for x in 0..ts {
                    let (cx, cy) = (tx * ts + x, ty * ts + y);
                    if cx >= x0
                        && cx < x0 + w
                        && cy >= y0
                        && cy < y0 + h
                        && cx < d.width()
                        && cy < d.height()
                    {
                        tile[((y * ts + x) * 4) as usize..][..4].copy_from_slice(&pixel(cx, cy));
                    }
                }
            }
            d.import_tile(layer, Channel::Color, TileCoord::new(tx, ty), &tile)
                .unwrap();
        }
    }
}

/// 4096² の、実物に近い合成の文書: 地・色の面・陰影（半透明・合成モード）・線画・ノイズ・マスク・グループ・調整・焼くもの（フィルター・半透明の塗りつぶし）。
fn big_document(n: u32) -> Document {
    use yolu_core::BlendMode as Blend;
    let mut d = Document::new(n, n).unwrap();
    d.set_source_budget_bytes(8192 * MIB).unwrap();
    let mut seed = 1u32;
    let base = d.add_layer("地").unwrap();
    paint_rect(&mut d, base, (0, 0, n, n), |x, y| {
        [(x / 17) as u8, (y / 19) as u8, 180, 255]
    });
    let mut flats = Vec::new();
    for i in 0..12u32 {
        let l = d.add_layer(&format!("色の面{i}")).unwrap();
        let (side, x0, y0) = (
            n * 5 / 12 + i * 20,
            (i * 311) % (n / 2),
            (i * 517) % (n / 2),
        );
        let c = [
            (40 + i * 17) as u8,
            (200 - i * 9) as u8,
            (90 + i * 11) as u8,
        ];
        paint_rect(&mut d, l, (x0, y0, side, side), |x, y| {
            if (x / 64 + y / 64) % 5 == 0 {
                [c[2], c[0], c[1], 255]
            } else {
                [c[0], c[1], c[2], 255]
            }
        });
        d.set_layer_opacity(l, f64::from(255 - (i * 7) as u8) / 255.0, false)
            .unwrap();
        if i % 3 == 0 {
            d.set_layer_blend_mode(l, Blend::Multiply).unwrap();
        }
        if i % 2 == 0 {
            d.add_layer_mask(l).unwrap();
            let ts = d.tile_size();
            for ty in 0..n.div_ceil(ts) {
                for tx in (0..n.div_ceil(ts)).step_by(2) {
                    let mut tile = vec![0u8; (ts * ts * 4) as usize];
                    for p in tile.as_chunks_mut::<4>().0 {
                        p[3] = ((ty * 7 + tx * 3) % 200) as u8
                    }
                    d.import_mask_tile(l, TileCoord::new(tx, ty), &tile)
                        .unwrap();
                }
            }
        }
        flats.push(l);
    }
    for i in 0..6u32 {
        let l = d.add_layer(&format!("陰影{i}")).unwrap();
        paint_rect(
            &mut d,
            l,
            (i * 200, i * 150, n * 3 / 4, n * 3 / 4),
            |x, y| {
                [
                    (x / 32) as u8,
                    30,
                    (y / 32) as u8,
                    (60 + (x / 40 + y / 40) % 120) as u8,
                ]
            },
        );
        d.set_layer_blend_mode(
            l,
            [Blend::Multiply, Blend::Screen, Blend::Overlay][(i % 3) as usize],
        )
        .unwrap();
    }
    for i in 0..4u32 {
        let l = d.add_layer(&format!("線画{i}")).unwrap();
        paint_rect(&mut d, l, (100 * i, 100 * i, n - 300, n - 300), |x, y| {
            if (x * 7 + y * 13 + i * 5) % 97 < 2 {
                [10, 10, 12, 255]
            } else {
                [0, 0, 0, 0]
            }
        });
    }
    for i in 0..2u32 {
        let l = d.add_layer(&format!("ノイズ{i}")).unwrap();
        paint_rect(&mut d, l, (300 * i, 300 * i, 1024, 1024), |_, _| {
            [noise(&mut seed), noise(&mut seed), noise(&mut seed), 255]
        });
    }
    for i in 0..3u32 {
        d.add_fill_layer(
            &format!("半透明{i}"),
            &[(
                Channel::Color,
                Rgba8::new(20 + i as u8, 40, 60, 40 + i as u8 * 20),
            )],
            None,
        )
        .unwrap();
    }
    for (i, l) in flats.iter().take(3).enumerate() {
        d.add_filter(
            *l,
            FilterTarget::Content,
            FilterSpec::new(EffectSettings::blur(3 + i as u32)),
        )
        .unwrap();
    }
    d.add_adjustment_layer("反転", AdjustmentSettings::invert(), None, None)
        .unwrap();
    d.add_adjustment_layer(
        "レベル",
        AdjustmentSettings::levels(
            20.0 / 255.0,
            230.0 / 255.0,
            1.37,
            10.0 / 255.0,
            240.0 / 255.0,
        )
        .unwrap(),
        None,
        None,
    )
    .unwrap();
    d.add_adjustment_layer(
        "色相",
        AdjustmentSettings::hue_saturation(-73.0, 0.42, -0.18).unwrap(),
        None,
        None,
    )
    .unwrap();
    for g in 0..3usize {
        let ids: Vec<LayerId> = flats[g * 4..g * 4 + 2].to_vec();
        d.group_layers(&ids, &format!("グループ{g}")).unwrap();
    }
    d
}

#[test]
#[ignore = "実物の大きさの測定（時間・ファイルの大きさ・メモリの山を出力する）"]
fn measure_a_real_size_document() {
    use std::time::Instant;
    let n = std::env::var("YOLU_MEASURE_SIDE")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(4096u32);
    let t = Instant::now();
    let d = big_document(n);
    let layers = d.layers().len();
    println!(
        "文書: {n}² / レイヤー {layers} / 作成 {:.1} s / 文書を持つメモリ {} MiB",
        t.elapsed().as_secs_f64(),
        proc_kib("VmRSS:") / 1024
    );
    let ctl = budget(256);
    let t = Instant::now();
    let plan = psd::plan_export(&d, &bake(), &ctl).unwrap();
    println!(
        "計画: {:.1} s / 注記 {} 件",
        t.elapsed().as_secs_f64(),
        plan.notes.len()
    );
    // 無圧縮も、RLE と同じように一時ファイルへ書いて測る（時間にディスクの書き込みを含める）。測ったらすぐ消す
    let path = std::env::temp_dir().join(format!("yolu-measure-{}.psd", std::process::id()));
    let t = Instant::now();
    let file = std::fs::File::create(&path).unwrap();
    let mut out = std::io::BufWriter::with_capacity(1 << 20, file);
    let raw = plan
        .write_psd(&d, &ctl, &mut out, Compression::Raw)
        .unwrap();
    std::io::Write::flush(&mut out).unwrap();
    drop(out);
    println!(
        "無圧縮（ファイルへ）: {} MiB / 書く {:.1} s",
        raw.bytes / (1024 * 1024),
        t.elapsed().as_secs_f64()
    );
    let _ = std::fs::remove_file(&path);
    // 書く出力を数えて捨てる場合（ディスクの書き込みを含まない）
    let mut null = Null { pos: 0, len: 0 };
    let t = Instant::now();
    plan.write_psd(&d, &ctl, &mut null, Compression::Raw)
        .unwrap();
    println!(
        "無圧縮（捨てる出力へ）: 書く {:.1} s",
        t.elapsed().as_secs_f64()
    );
    // RLE で、一時ファイルへ流して書く
    let before = proc_kib("VmRSS:");
    let _ = std::fs::write("/proc/self/clear_refs", "5");
    let t = Instant::now();
    let file = std::fs::File::create(&path).unwrap();
    let mut out = std::io::BufWriter::with_capacity(1 << 20, file);
    let written = plan
        .write_psd(&d, &ctl, &mut out, Compression::Rle)
        .unwrap();
    std::io::Write::flush(&mut out).unwrap();
    drop(out);
    let secs = t.elapsed().as_secs_f64();
    let peak = proc_kib("VmHWM:");
    println!(
        "RLE: {} MiB（無圧縮の {:.0}%）/ 書く {:.1} s / メモリの山 {} MiB（書く前 {} MiB、増えた分 {} MiB）",
        written.bytes / (1024 * 1024),
        100.0 * written.bytes as f64 / raw.bytes as f64,
        secs,
        peak / 1024,
        before / 1024,
        peak.saturating_sub(before) / 1024
    );
    assert_eq!(std::fs::metadata(&path).unwrap().len(), written.bytes);
    let t = Instant::now();
    let _ = std::fs::write("/proc/self/clear_refs", "5");
    let verified = psd::verify_stream(
        &mut std::io::BufReader::new(std::fs::File::open(&path).unwrap()),
        None,
    )
    .unwrap();
    println!(
        "流して確かめる: {:.1} s / メモリの山 {} MiB / レイヤー {}",
        t.elapsed().as_secs_f64(),
        proc_kib("VmHWM:") / 1024,
        verified.layers
    );
    assert_eq!(verified.layers, written.layers);
    // 書いたバイト列との照合（CRC-32）
    let t = Instant::now();
    let _ = std::fs::write("/proc/self/clear_refs", "5");
    assert!(written
        .checksum
        .matches(&mut std::fs::File::open(&path).unwrap(), None)
        .unwrap());
    println!("バイト列の照合: {:.1} s", t.elapsed().as_secs_f64());
    // 読み込み直して合成を照らす
    let t = Instant::now();
    let imported = match psd::import_copy(
        &mut std::io::BufReader::new(std::fs::File::open(&path).unwrap()),
        &psd::CopyOptions {
            source_budget: 8192 * MIB,
            cancel: None,
        },
    )
    .unwrap()
    {
        psd::CopyOutcome::Imported(i) => i,
        psd::CopyOutcome::Refused(why) => panic!("{}", why.message()),
    };
    println!(
        "取り込み直す: {:.1} s / レイヤー {}",
        t.elapsed().as_secs_f64(),
        imported.document.layers().len()
    );
    let _ = std::fs::remove_file(&path);
    let want = d.composite_channel(Channel::Color, d.bounds()).unwrap();
    let got = imported
        .document
        .composite_channel(Channel::Color, imported.document.bounds())
        .unwrap();
    let max = want
        .iter()
        .zip(&got)
        .map(|(a, b)| a.abs_diff(*b))
        .max()
        .unwrap_or(0);
    let off = want.iter().zip(&got).filter(|(a, b)| a != b).count();
    println!("合成の差: 最大 {max} / 違うバイト {off}");
    assert_eq!(max, 0, "読み戻した合成は、書き出した文書の合成と同じ");
}
