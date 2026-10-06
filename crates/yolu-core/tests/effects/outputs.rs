//! 層・マスク・グループの領域の出力（`layer_output`・`mask_output`・`group_output`）: 合成と同じ評価の道を通り、1 画素ずつの読み
//! （`layer_output_pixel`・`mask_output_hide`）と矩形・行の並びによらず同じ値になる。文書は変えない。
use crate::attach_support;
use attach_support::*;
use std::sync::atomic::AtomicBool;
use yolu_core::{
    Channel, CoreError, Document, EffectSettings, FilterSpec, FilterTarget, LayerId, Rect, Rgba8,
    RowOrder,
};

fn all(doc: &Document) -> Rect {
    Rect::new(0, 0, doc.width(), doc.height())
}

/// 下から上の行の並びの全面から、矩形（タイルの境をまたぐ）の部分を切り出す。
fn crop(whole: &[u8], width: u32, rect: Rect, bytes: usize) -> Vec<u8> {
    let mut out = Vec::new();
    for y in rect.y..rect.y + rect.height {
        let start = (y as usize * width as usize + rect.x as usize) * bytes;
        out.extend_from_slice(&whole[start..start + rect.width as usize * bytes]);
    }
    out
}

/// 行を上下に入れ替える。
fn flipped(bytes: &[u8], width: u32, bytes_per_pixel: usize) -> Vec<u8> {
    bytes
        .chunks_exact(width as usize * bytes_per_pixel)
        .rev()
        .flatten()
        .copied()
        .collect()
}

/// 完全に透明な画素の RGB を 0 にそろえる（合成は透明の下の色を持たない）。
fn without_hidden_colour(mut bytes: Vec<u8>) -> Vec<u8> {
    for pixel in bytes.as_chunks_mut::<4>().0 {
        if pixel[3] == 0 {
            pixel.fill(0);
        }
    }
    bytes
}

fn blur(doc: &mut Document, layer: LayerId, channel: Channel) {
    doc.add_filter(
        layer,
        FilterTarget::Content,
        FilterSpec::new(EffectSettings::blur(3)).channels(&[channel]),
    )
    .unwrap();
}

/// 保存した画素・フィルターを通した画素・塗りつぶしの値・フィルターつきの塗りつぶしの出力が、1 画素ずつの読みと全面で同じ値で、
/// 矩形・行の並びを替えても同じ値になる。
#[test]
fn a_layer_output_is_the_pixel_reads_in_any_rect_and_row_order() {
    let (mut doc, l) = world();
    let (base, mid, top, fill) = (l[0], l[1], l[2], l[3]);
    blur(&mut doc, base, Channel::Color);
    blur(&mut doc, fill, Channel::Color);
    let rect = Rect::new(5, 3, 19, 17);
    for (name, id, channel) in [
        ("フィルターつきのラスター", base, Channel::Color),
        ("保存した画素のラスター", top, Channel::Color),
        (
            "フィルターが別のチャンネルだけのラスター",
            base,
            Channel::Height,
        ),
        ("面の無いチャンネル", mid, Channel::Color),
        ("フィルターつきの塗りつぶし", fill, Channel::Color),
        ("値だけの塗りつぶし", fill, Channel::Height),
        ("値の無い塗りつぶし", fill, Channel::Roughness),
    ] {
        let whole = doc.layer_output(id, channel, all(&doc)).unwrap();
        for y in 0..doc.height() {
            for x in 0..doc.width() {
                let at = (y as usize * doc.width() as usize + x as usize) * 4;
                assert_eq!(
                    Rgba8::from_slice(&whole[at..at + 4]),
                    doc.layer_output_pixel(id, channel, x, y).unwrap(),
                    "{name} ({x}, {y})"
                );
            }
        }
        let part = doc.layer_output(id, channel, rect).unwrap();
        assert_eq!(part, crop(&whole, doc.width(), rect, 4), "{name}: 矩形");
        let mut top_down = vec![0u8; part.len()];
        doc.layer_output_into(id, channel, rect, &mut top_down, RowOrder::TopDown, None)
            .unwrap();
        assert_eq!(top_down, flipped(&part, rect.width, 4), "{name}: 上から下");
    }
}

/// 層が 1 枚だけの文書では、層の出力は合成そのもの（同じ評価・同じキャッシュを通る）。
#[test]
fn a_lone_layers_output_is_the_composite() {
    for filtered in [false, true] {
        let mut doc = Document::with_tile_size(W, H, 8).unwrap();
        doc.set_filter_block_pixels(16).unwrap();
        let id = doc.add_layer("a").unwrap();
        paint(&mut doc, id, Channel::Color, 7);
        if filtered {
            blur(&mut doc, id, Channel::Color);
        }
        assert_eq!(
            without_hidden_colour(doc.layer_output(id, Channel::Color, all(&doc)).unwrap()),
            whole(&doc, Channel::Color),
            "フィルター {filtered}"
        );
    }
    // 塗りつぶし（半透明の値）も合成と同じ
    let mut doc = Document::with_tile_size(W, H, 8).unwrap();
    let id = doc
        .add_fill_layer("f", &[(Channel::Color, Rgba8::new(10, 20, 30, 77))], None)
        .unwrap();
    assert_eq!(
        doc.layer_output(id, Channel::Color, all(&doc)).unwrap(),
        whole(&doc, Channel::Color)
    );
}

/// マスクの出力: フィルターが無ければ隠す量そのもの、有効なフィルターがあれば通した値。1 画素ずつの読みと同じで、反転・無効・濃度は
/// 掛けない。
#[test]
fn a_mask_output_is_the_hide_amount_through_its_filters_before_invert_and_density() {
    let (mut doc, l) = world();
    let top = l[2];
    doc.add_layer_mask(top).unwrap();
    for x in 0..doc.width() {
        doc.set_mask_pixel(top, x, 4, (x * 6) as u8).unwrap();
    }
    let plain = doc.mask_output(top, all(&doc)).unwrap();
    assert_eq!(plain.len(), (W * H) as usize);
    for y in 0..H {
        for x in 0..W {
            assert_eq!(
                plain[(y * W + x) as usize],
                doc.mask_output_hide(top, x, y).unwrap()
            );
        }
    }
    assert_eq!(plain[(4 * W + 3) as usize], 18);
    // 反転・無効・濃度は出力の値を変えない
    doc.set_layer_mask_inverted(top, true).unwrap();
    doc.set_layer_mask_enabled(top, false).unwrap();
    doc.set_layer_mask_density(top, 0.25, false).unwrap();
    assert_eq!(doc.mask_output(top, all(&doc)).unwrap(), plain);
    // フィルター（反転）を通した値。無効の段は通さない
    let f = doc
        .add_filter(
            top,
            FilterTarget::Mask,
            FilterSpec::new(EffectSettings::invert()),
        )
        .unwrap();
    let inverted = doc.mask_output(top, all(&doc)).unwrap();
    assert_eq!(inverted.len(), plain.len());
    for y in 0..H {
        for x in 0..W {
            let i = (y * W + x) as usize;
            assert_eq!(inverted[i], doc.mask_output_hide(top, x, y).unwrap());
            assert_eq!(inverted[i], 255 - plain[i], "({x}, {y})");
        }
    }
    let rect = Rect::new(2, 1, 21, 9);
    assert_eq!(
        doc.mask_output(top, rect).unwrap(),
        crop(&inverted, W, rect, 1)
    );
    let mut top_down = vec![0u8; (rect.width * rect.height) as usize];
    doc.mask_output_into(top, rect, &mut top_down, RowOrder::TopDown, None)
        .unwrap();
    assert_eq!(
        top_down,
        flipped(&crop(&inverted, W, rect, 1), rect.width, 1)
    );
    doc.set_filter_enabled(top, f, false).unwrap_or(());
}

/// グループの出力は、子を透明から重ねた合成。グループ 1 つだけの文書では合成そのもので、グループ自身の不透明度・モード・マスクは
/// 掛けない。子のフィルターも通る。
#[test]
fn a_group_output_is_its_children_composited_from_transparent() {
    let mut doc = Document::with_tile_size(W, H, 8).unwrap();
    doc.set_filter_block_pixels(16).unwrap();
    let a = doc.add_layer("a").unwrap();
    paint(&mut doc, a, Channel::Color, 1);
    let b = doc.add_layer("b").unwrap();
    paint(&mut doc, b, Channel::Color, 2);
    doc.set_layer_blend_mode(b, yolu_core::BlendMode::Multiply)
        .unwrap();
    doc.set_layer_opacity(b, 0.6, false).unwrap();
    blur(&mut doc, a, Channel::Color);
    let c = doc.add_layer("c").unwrap();
    paint(&mut doc, c, Channel::Color, 3);
    doc.set_layer_clipping(c, true).unwrap();
    let g = doc.group_layers(&[a, b, c], "g").unwrap();
    let expected = whole(&doc, Channel::Color);
    let out = doc.group_output(g, Channel::Color, all(&doc)).unwrap();
    assert_eq!(out, expected, "グループだけの文書の合成と同じ");
    // グループ自身の不透明度・モード・マスク・クリッピングは出力に掛からない
    doc.set_layer_opacity(g, 0.3, false).unwrap();
    doc.set_layer_blend_mode(g, yolu_core::BlendMode::Screen)
        .unwrap();
    doc.add_layer_mask(g).unwrap();
    doc.set_mask_pixel(g, 4, 4, 255).unwrap();
    doc.set_layer_clipping(g, true).unwrap();
    assert_eq!(doc.group_output(g, Channel::Color, all(&doc)).unwrap(), out);
    // 別のチャンネルは、そのチャンネルの面の合成（Height には何も無い）
    assert!(doc
        .group_output(g, Channel::Height, all(&doc))
        .unwrap()
        .iter()
        .all(|&b| b == 0));
    // 矩形・行の並び
    let rect = Rect::new(3, 2, 22, 19);
    let part = doc.group_output(g, Channel::Color, rect).unwrap();
    assert_eq!(part, crop(&out, W, rect, 4));
    let mut top_down = vec![0u8; part.len()];
    doc.group_output_into(
        g,
        Channel::Color,
        rect,
        &mut top_down,
        RowOrder::TopDown,
        None,
    )
    .unwrap();
    assert_eq!(top_down, flipped(&part, rect.width, 4));
}

/// 入れ子のグループ（通過・分離）も、合成と同じ式で重なる。
#[test]
fn a_nested_group_output_matches_the_composite_of_the_same_stack() {
    let (mut doc, l) = world();
    let (base, mid, top) = (l[0], l[1], l[2]);
    let inner = doc.group_layers(&[mid, top], "inner").unwrap();
    doc.set_layer_blend_mode(inner, yolu_core::BlendMode::Overlay)
        .unwrap();
    let outer = doc.group_layers(&[base, inner], "outer").unwrap();
    let fill = l[3];
    let expected = {
        // 同じ層だけの文書（塗りつぶしを外した文書）の合成
        let mut only = doc.capture_snapshot().unwrap();
        only.remove_layer(fill).unwrap();
        whole(&only, Channel::Color)
    };
    assert_eq!(
        doc.group_output(outer, Channel::Color, all(&doc)).unwrap(),
        expected
    );
}

/// 出力を読んでも文書は変わらず（版・Undo の段）、同じ読みは何度でも同じ値。取消の旗・矩形・種類の誤りは断る。
#[test]
fn outputs_leave_the_document_alone_and_refuse_what_they_cannot_read() {
    let (mut doc, l) = world();
    let (base, top, fill) = (l[0], l[2], l[3]);
    blur(&mut doc, base, Channel::Color);
    let g = doc.group_layers(&[base], "g").unwrap();
    doc.add_layer_mask(top).unwrap();
    let (revision, undo, bytes) = (doc.revision(), doc.undo_count(), doc.allocated_bytes());
    let first = doc.layer_output(base, Channel::Color, all(&doc)).unwrap();
    let _ = doc.mask_output(top, all(&doc)).unwrap();
    let group = doc.group_output(g, Channel::Color, all(&doc)).unwrap();
    assert_eq!(
        doc.layer_output(base, Channel::Color, all(&doc)).unwrap(),
        first
    );
    assert_eq!(
        doc.group_output(g, Channel::Color, all(&doc)).unwrap(),
        group
    );
    assert_eq!(
        (doc.revision(), doc.undo_count(), doc.allocated_bytes()),
        (revision, undo, bytes)
    );
    // 取消の旗が立っていれば、何も読まずに断る
    let stop = AtomicBool::new(true);
    let mut out = vec![0u8; (W * H * 4) as usize];
    assert_eq!(
        doc.layer_output_into(
            base,
            Channel::Color,
            all(&doc),
            &mut out,
            RowOrder::BottomUp,
            Some(&stop)
        ),
        Err(CoreError::Cancelled)
    );
    assert_eq!(
        doc.group_output_into(
            g,
            Channel::Color,
            all(&doc),
            &mut out,
            RowOrder::BottomUp,
            Some(&stop)
        ),
        Err(CoreError::Cancelled)
    );
    let mut mask_out = vec![0u8; (W * H) as usize];
    assert_eq!(
        doc.mask_output_into(
            top,
            all(&doc),
            &mut mask_out,
            RowOrder::BottomUp,
            Some(&stop)
        ),
        Err(CoreError::Cancelled)
    );
    // 種類・矩形・大きさの誤り
    assert!(matches!(
        doc.layer_output(g, Channel::Color, all(&doc)),
        Err(CoreError::Unsupported(_))
    ));
    assert!(matches!(
        doc.group_output(fill, Channel::Color, all(&doc)),
        Err(CoreError::Unsupported(_))
    ));
    assert!(matches!(
        doc.mask_output(fill, all(&doc)),
        Err(CoreError::Unsupported(_))
    ));
    assert!(matches!(
        doc.layer_output(base, Channel::Color, Rect::new(30, 0, 20, 4)),
        Err(CoreError::InvalidArgument(_))
    ));
    assert!(matches!(
        doc.layer_output_into(
            base,
            Channel::Color,
            all(&doc),
            &mut out[..8],
            RowOrder::BottomUp,
            None
        ),
        Err(CoreError::InvalidArgument(_))
    ));
    assert!(doc
        .layer_output(base, Channel::Color, Rect::new(0, 0, 0, 0))
        .unwrap()
        .is_empty());
}

/// 調整の設定だけを替えた読むだけの写し: 写しの合成は替えた設定で、元の文書（設定・版・合成）は変わらない。調整でない層・無い層・描いている最中は断る。
#[test]
fn a_copy_with_replaced_adjustments_composites_with_the_new_settings_and_leaves_the_original() {
    let (mut doc, l) = world();
    let (base, top) = (l[0], l[2]);
    let adjustment = doc
        .add_adjustment_layer(
            "調整",
            yolu_core::AdjustmentSettings::levels(0.3, 1.0, 1.0, 0.0, 1.0).unwrap(),
            None,
            None,
        )
        .unwrap();
    let (revision, undo, composite) = (
        doc.revision(),
        doc.undo_count(),
        whole(&doc, Channel::Color),
    );
    let rounded = yolu_core::AdjustmentSettings::levels(76.0 / 255.0, 1.0, 1.0, 0.0, 1.0).unwrap();
    let copy = doc
        .with_adjustments_replaced(&[(adjustment, rounded.clone())])
        .unwrap();
    assert_eq!(
        *copy.layer(adjustment).unwrap().adjustment().unwrap(),
        rounded
    );
    assert_ne!(
        whole(&copy, Channel::Color),
        composite,
        "替えた設定で合成する"
    );
    // 同じ設定の文書を一から組んだ合成と同じ
    let (mut direct, d) = world();
    direct
        .add_adjustment_layer("調整", rounded.clone(), None, None)
        .unwrap();
    let _ = d;
    assert_eq!(whole(&copy, Channel::Color), whole(&direct, Channel::Color));
    assert_eq!(
        (
            doc.revision(),
            doc.undo_count(),
            whole(&doc, Channel::Color)
        ),
        (revision, undo, composite),
        "元は変わらない"
    );
    assert!(matches!(
        doc.with_adjustments_replaced(&[(base, rounded.clone())]),
        Err(CoreError::Unsupported(_))
    ));
    assert!(matches!(
        doc.with_adjustments_replaced(&[(LayerId(7), rounded.clone())]),
        Err(CoreError::LayerNotFound)
    ));
    let stroke = doc
        .begin_stroke(top, &yolu_core::BrushSettings::default())
        .unwrap();
    assert!(matches!(
        doc.with_adjustments_replaced(&[(adjustment, rounded.clone())]),
        Err(CoreError::StrokeActive)
    ));
    doc.cancel_stroke(stroke);
}

/// `contributes`: 合成の計画に出る層か。見えない・不透明度 0・そのチャンネルに中身が無い層と、子の計画が空のグループは出ない。
#[test]
fn contributes_says_whether_the_composite_keeps_the_layer() {
    let (mut doc, l) = world();
    let (base, mid, top, fill) = (l[0], l[1], l[2], l[3]);
    // ラスターは面のあるチャンネルで出る（土台は Color と Height、中は Color（空）と Height で、Roughness の面は無い）
    assert!(doc.contributes(base, Channel::Color).unwrap());
    assert!(!doc.contributes(mid, Channel::Roughness).unwrap());
    assert!(doc.contributes(mid, Channel::Height).unwrap());
    // 塗りつぶしは値のあるチャンネルで出る
    assert!(doc.contributes(fill, Channel::Color).unwrap());
    assert!(!doc.contributes(fill, Channel::Roughness).unwrap());
    // 見えない・不透明度 0
    doc.set_layer_visible(top, false).unwrap();
    assert!(!doc.contributes(top, Channel::Color).unwrap());
    doc.set_layer_visible(top, true).unwrap();
    doc.set_layer_opacity(top, 0.0, false).unwrap();
    assert!(!doc.contributes(top, Channel::Color).unwrap());
    doc.set_layer_opacity(top, 1.0, false).unwrap();
    // グループ: 子の計画が空なら出ない（Roughness の面が無い中だけのグループ）。1 つでも出る子があれば出る
    let only_mid = doc.group_layers(&[mid], "中だけ").unwrap();
    assert!(!doc.contributes(only_mid, Channel::Roughness).unwrap());
    assert!(doc.contributes(only_mid, Channel::Height).unwrap());
    let empty = doc.add_group("空", None).unwrap();
    assert!(!doc.contributes(empty, Channel::Roughness).unwrap());
    doc.set_layer_visible(only_mid, false).unwrap();
    assert!(!doc.contributes(only_mid, Channel::Height).unwrap());
    assert!(matches!(
        doc.contributes(LayerId(7), Channel::Color),
        Err(CoreError::LayerNotFound)
    ));
}
