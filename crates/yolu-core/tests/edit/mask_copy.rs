//! レイヤーマスクの丸ごとのコピー・別のレイヤーへの移動・置き換え、黒のマスクの追加、マスクへの画像の貼り付け。
//! どれも 1 回の Undo で戻り、断ったら何も変えない。

use yolu_core::generator::{self, anchor::ReadMode, Settings};
use yolu_core::{
    AnchorIssueKind, AnchorPlacement, Channel, CoreError, Document, EffectSettings, FilterSpec,
    FilterTarget, HistoryKind, LayerId, LayerLocks, PixelClipboard, Reason, Rgba8, SelectionMask,
};

const W: u32 = 41;
const H: u32 = 35;
const TILE: u32 = 16;

fn doc() -> Document {
    Document::with_tile_size(W, H, TILE).unwrap()
}

/// 色のある 1 色のレイヤー（全面）。
fn full(d: &mut Document, name: &str, c: Rgba8) -> LayerId {
    let id = d.add_layer(name).unwrap();
    for y in 0..H {
        for x in 0..W {
            d.set_pixel(id, x, y, c).unwrap();
        }
    }
    id
}

/// 2 つのレイヤー（a が下、b が上）。履歴は空。
fn two() -> (Document, LayerId, LayerId) {
    let mut d = doc();
    let a = full(&mut d, "a", Rgba8::new(255, 0, 0, 255));
    let b = full(&mut d, "b", Rgba8::new(0, 0, 255, 255));
    d.clear_history().unwrap();
    (d, a, b)
}

fn hide_at(d: &Document, id: LayerId, x: u32, y: u32) -> u8 {
    d.layer(id)
        .unwrap()
        .mask()
        .unwrap()
        .surface()
        .pixel(x, y)
        .unwrap()
        .a
}

/// 1 つのマスクの、見た目に効く全部（画素・有効・反転・濃度・フィルターの ID・Anchor の ID）。
#[derive(Debug, PartialEq)]
struct MaskSnap {
    pixels: Vec<u8>,
    enabled: bool,
    inverted: bool,
    density_bits: u64,
    filters: Vec<u128>,
    anchor: Option<u128>,
}

/// レイヤーごとのマスクと、履歴の段の数。
#[derive(Debug, PartialEq)]
struct Snap {
    layers: Vec<(LayerId, Option<MaskSnap>)>,
    undo: usize,
    redo: usize,
}

fn snap(d: &Document) -> Snap {
    Snap {
        layers: d
            .layers()
            .iter()
            .map(|l| {
                (
                    l.id(),
                    l.mask().map(|m| MaskSnap {
                        pixels: m.surface().canvas_bytes().unwrap(),
                        enabled: m.enabled(),
                        inverted: m.inverted(),
                        density_bits: m.density().to_bits(),
                        filters: m.filters().iter().map(|f| f.id().0).collect(),
                        anchor: m.anchor().map(|a| a.id().0),
                    }),
                )
            })
            .collect(),
        undo: d.undo_count(),
        redo: d.redo_count(),
    }
}

/// 編集が 1 回の Undo で、Undo で前の状態、Redo で後の状態に戻ること。
fn one_undo_step(d: &mut Document, what: &str, edit: impl FnOnce(&mut Document)) {
    let before = snap(d);
    edit(d);
    let after = snap(d);
    assert_eq!(after.undo, before.undo + 1, "{what}: 1 回の Undo");
    assert_ne!(after.layers, before.layers, "{what}: 何かが変わる");
    assert!(d.undo().unwrap(), "{what}");
    let back = snap(d);
    assert_eq!(back.layers, before.layers, "{what}: Undo で前");
    assert_eq!(back.undo, before.undo);
    assert!(d.redo().unwrap(), "{what}");
    assert_eq!(snap(d), after, "{what}: Redo で後");
}

fn last_kind(d: &Document) -> HistoryKind {
    d.history().nth(d.undo_count() - 1).unwrap()
}

// ───────── 黒のマスク ─────────

#[test]
fn a_black_mask_hides_everything_in_one_undo_step() {
    let (mut d, a, _b) = two();
    one_undo_step(&mut d, "黒のマスク", |d| {
        d.add_black_layer_mask(a).unwrap()
    });
    assert_eq!(last_kind(&d), HistoryKind::AddMask);
    // 全部隠れる: a の下に何も無いので、b の下の a は見えない
    for (x, y) in [(0, 0), (W - 1, H - 1), (17, 20)] {
        assert_eq!(hide_at(&d, a, x, y), 255);
    }
    assert_eq!(
        d.add_layer_mask(a),
        Err(CoreError::Unsupported(Reason::LayerAlreadyHasMask))
    );
    assert_eq!(
        d.add_black_layer_mask(a),
        Err(CoreError::Unsupported(Reason::LayerAlreadyHasMask))
    );
}

#[test]
fn a_black_mask_is_the_same_pixels_as_adding_a_mask_and_filling_it() {
    let (mut d, a, b) = two();
    d.add_black_layer_mask(a).unwrap();
    d.add_layer_mask(b).unwrap();
    d.fill_mask(b, 1.0, None, false).unwrap();
    let black = |id| {
        d.layer(id)
            .unwrap()
            .mask()
            .unwrap()
            .surface()
            .canvas_bytes()
            .unwrap()
    };
    assert_eq!(black(a), black(b));
    // 合成の結果も同じ
    let image = d.composite_pixel(Channel::Color, 3, 3).unwrap();
    assert_eq!(image.a, 0, "両方とも隠れて何も見えない");
}

#[test]
fn a_black_mask_holds_uniform_tiles_only_and_ignores_the_selection() {
    let (mut d, a, _b) = two();
    d.set_selection(Some(SelectionMask::rectangle(&d, 0, 0, 3, 3)))
        .unwrap();
    d.clear_history().unwrap();
    d.add_black_layer_mask(a).unwrap();
    let surface = d.layer(a).unwrap().mask().unwrap().surface();
    assert_eq!(
        surface.allocated_bytes(),
        4 * surface.tile_count() as u64,
        "どのタイルも一様（4 バイト）"
    );
    assert_eq!(
        surface.tile_count() as u32,
        3 * 3,
        "41 × 35 は 16 のタイルで 3 × 3"
    );
    assert_eq!(hide_at(&d, a, W - 1, H - 1), 255, "選択の外も隠す");
}

#[test]
fn a_black_mask_is_refused_when_locked_and_changes_nothing() {
    let (mut d, a, _b) = two();
    d.set_layer_locks(a, LayerLocks::ALL).unwrap();
    let before = snap(&d);
    assert!(matches!(
        d.add_black_layer_mask(a),
        Err(CoreError::LayerLocked { .. })
    ));
    assert_eq!(snap(&d), before);
}

// ───────── コピーと貼り付け ─────────

/// 設定の揃ったマスク（途中の濃さ・反転・濃度・フィルター・Anchor つき）を a に付ける。
fn decorated_mask(d: &mut Document, id: LayerId) {
    d.add_layer_mask(id).unwrap();
    d.fill_mask(
        id,
        0.5,
        Some(&SelectionMask::rectangle(d, 2, 2, 20, 20)),
        false,
    )
    .unwrap();
    d.set_layer_mask_inverted(id, true).unwrap();
    d.set_layer_mask_density(id, 0.75, false).unwrap();
    d.add_filter(
        id,
        FilterTarget::Mask,
        FilterSpec::new(EffectSettings::blur(2)),
    )
    .unwrap();
}

#[test]
fn copying_and_pasting_a_mask_replaces_the_target_in_one_undo_step() {
    let (mut d, a, b) = two();
    decorated_mask(&mut d, a);
    d.add_anchor(a, AnchorPlacement::Mask, None, None).unwrap();
    d.add_layer_mask(b).unwrap();
    d.fill_mask(b, 1.0, None, false).unwrap();
    d.clear_history().unwrap();
    let copy = d.copy_mask(a).unwrap();
    assert_eq!(snap(&d).undo, 0, "コピーは文書を変えない");
    assert_eq!(
        (
            copy.enabled(),
            copy.inverted(),
            copy.density(),
            copy.filter_count()
        ),
        (true, true, 0.75, 1)
    );
    one_undo_step(&mut d, "貼る", |d| d.set_masks(&[b], &copy).unwrap());
    assert_eq!(last_kind(&d), HistoryKind::PasteMask);
    let (ma, mb) = (
        d.layer(a).unwrap().mask().unwrap(),
        d.layer(b).unwrap().mask().unwrap(),
    );
    assert_eq!(
        ma.surface().canvas_bytes().unwrap(),
        mb.surface().canvas_bytes().unwrap(),
        "面が同じ"
    );
    assert_eq!(
        (mb.enabled(), mb.inverted(), mb.density()),
        (true, true, 0.75)
    );
    assert_eq!(mb.filters().len(), 1);
    assert_ne!(
        mb.filters()[0].id(),
        ma.filters()[0].id(),
        "フィルターの ID は重ならない"
    );
    assert!(ma.anchor().is_some(), "元の Anchor はそのまま");
    assert!(mb.anchor().is_none(), "Anchor は写さない");
}

#[test]
fn pasting_a_mask_keeps_the_anchor_of_the_mask_it_replaces() {
    let (mut d, a, b) = two();
    decorated_mask(&mut d, a);
    d.add_layer_mask(b).unwrap();
    let anchor = d.add_anchor(b, AnchorPlacement::Mask, None, None).unwrap();
    d.clear_history().unwrap();
    let copy = d.copy_mask(a).unwrap();
    d.set_masks(&[b], &copy).unwrap();
    assert_eq!(
        d.layer(b).unwrap().mask().unwrap().anchor().map(|x| x.id()),
        Some(anchor)
    );
}

#[test]
fn pasting_to_a_layer_without_a_mask_adds_one() {
    let (mut d, a, b) = two();
    decorated_mask(&mut d, a);
    d.clear_history().unwrap();
    let copy = d.copy_mask(a).unwrap();
    one_undo_step(&mut d, "マスクの無い先へ", |d| {
        d.set_masks(&[b], &copy).unwrap()
    });
    assert!(d.layer(b).unwrap().mask().is_some());
    d.undo().unwrap();
    assert!(d.layer(b).unwrap().mask().is_none());
}

#[test]
fn pasting_to_several_layers_is_one_undo_step() {
    let mut d = doc();
    let a = full(&mut d, "a", Rgba8::new(255, 0, 0, 255));
    let b = full(&mut d, "b", Rgba8::new(0, 255, 0, 255));
    let c = full(&mut d, "c", Rgba8::new(0, 0, 255, 255));
    decorated_mask(&mut d, a);
    d.add_black_layer_mask(c).unwrap();
    d.clear_history().unwrap();
    let copy = d.copy_mask(a).unwrap();
    one_undo_step(&mut d, "3 つへ", |d| {
        d.set_masks(&[b, c, b], &copy).unwrap()
    });
    assert_eq!(hide_at(&d, b, 5, 5), hide_at(&d, a, 5, 5));
    assert_eq!(hide_at(&d, c, 5, 5), hide_at(&d, a, 5, 5), "置き換えた");
    assert_eq!(
        d.layer(b).unwrap().mask().unwrap().filters().len(),
        1,
        "同じレイヤーを重ねて指しても 1 回だけ"
    );
    d.undo().unwrap();
    assert_eq!(hide_at(&d, c, 5, 5), 255, "c は黒のマスクに戻る");
    assert!(d.layer(b).unwrap().mask().is_none());
}

#[test]
fn a_locked_target_refuses_the_whole_paste_and_changes_nothing() {
    let mut d = doc();
    let a = full(&mut d, "a", Rgba8::new(255, 0, 0, 255));
    let b = full(&mut d, "b", Rgba8::new(0, 255, 0, 255));
    let c = full(&mut d, "c", Rgba8::new(0, 0, 255, 255));
    decorated_mask(&mut d, a);
    d.set_layer_locks(c, LayerLocks::ALL).unwrap();
    d.clear_history().unwrap();
    let copy = d.copy_mask(a).unwrap();
    let before = snap(&d);
    assert!(matches!(
        d.set_masks(&[b, c], &copy),
        Err(CoreError::LayerLocked { .. })
    ));
    assert_eq!(snap(&d), before, "b にも貼られていない");
    assert_eq!(
        d.set_masks(&[LayerId(9999)], &copy),
        Err(CoreError::LayerNotFound)
    );
    assert_eq!(snap(&d), before);
}

#[test]
fn a_mask_from_a_different_canvas_size_is_refused() {
    let (mut d, a, _b) = two();
    d.add_layer_mask(a).unwrap();
    let copy = d.copy_mask(a).unwrap();
    assert_eq!(copy.size(), (W, H));
    let mut other = Document::with_tile_size(W + 1, H, TILE).unwrap();
    let t = other.add_layer("t").unwrap();
    other.clear_history().unwrap();
    assert_eq!(
        other.set_masks(&[t], &copy),
        Err(CoreError::Unsupported(Reason::MaskSizeDiffersFromCanvas))
    );
    assert!(other.layer(t).unwrap().mask().is_none());
    assert!(!other.can_undo());
}

#[test]
fn copying_without_a_mask_is_refused() {
    let (d, a, _b) = two();
    assert_eq!(
        d.copy_mask(a).unwrap_err(),
        CoreError::Unsupported(Reason::LayerHasNoMask)
    );
}

// ───────── 移動 ─────────

#[test]
fn moving_a_mask_takes_the_anchor_along_in_one_undo_step() {
    let (mut d, a, b) = two();
    decorated_mask(&mut d, b);
    let anchor = d.add_anchor(b, AnchorPlacement::Mask, None, None).unwrap();
    d.clear_history().unwrap();
    let source = d
        .layer(b)
        .unwrap()
        .mask()
        .unwrap()
        .surface()
        .canvas_bytes()
        .unwrap();
    one_undo_step(&mut d, "移動", |d| d.move_mask(b, a).unwrap());
    assert_eq!(last_kind(&d), HistoryKind::MoveMask);
    assert!(d.layer(b).unwrap().mask().is_none(), "元からは無くなる");
    let m = d.layer(a).unwrap().mask().unwrap();
    assert_eq!(m.surface().canvas_bytes().unwrap(), source);
    assert_eq!(
        (m.inverted(), m.density(), m.filters().len()),
        (true, 0.75, 1)
    );
    assert_eq!(
        m.anchor().map(|x| x.id()),
        Some(anchor),
        "Anchor は同じ ID のまま付いていく"
    );
    assert!(d.find_anchor(anchor).is_some());
}

#[test]
fn moving_onto_a_layer_with_a_mask_replaces_it_and_undo_brings_it_back() {
    let (mut d, a, b) = two();
    decorated_mask(&mut d, b);
    d.add_black_layer_mask(a).unwrap();
    let old_a = d
        .layer(a)
        .unwrap()
        .mask()
        .unwrap()
        .surface()
        .canvas_bytes()
        .unwrap();
    d.clear_history().unwrap();
    d.move_mask(b, a).unwrap();
    assert_ne!(
        d.layer(a)
            .unwrap()
            .mask()
            .unwrap()
            .surface()
            .canvas_bytes()
            .unwrap(),
        old_a
    );
    d.undo().unwrap();
    assert_eq!(
        d.layer(a)
            .unwrap()
            .mask()
            .unwrap()
            .surface()
            .canvas_bytes()
            .unwrap(),
        old_a,
        "置き換えられたマスクが戻る"
    );
    assert!(d.layer(b).unwrap().mask().is_some());
}

#[test]
fn moving_is_refused_without_a_mask_on_a_lock_and_during_a_stroke() {
    let (mut d, a, b) = two();
    assert_eq!(
        d.move_mask(a, b),
        Err(CoreError::Unsupported(Reason::LayerHasNoMask))
    );
    d.add_layer_mask(a).unwrap();
    d.clear_history().unwrap();
    let before = snap(&d);
    d.set_layer_locks(b, LayerLocks::ALL).unwrap();
    let locked = snap(&d);
    assert!(
        matches!(d.move_mask(a, b), Err(CoreError::LayerLocked { .. })),
        "行き先のロック"
    );
    assert_eq!(snap(&d), locked);
    d.undo().unwrap();
    d.set_layer_locks(a, LayerLocks::ALL).unwrap();
    let locked = snap(&d);
    assert!(
        matches!(d.move_mask(a, b), Err(CoreError::LayerLocked { .. })),
        "元のロック"
    );
    assert_eq!(snap(&d), locked);
    d.undo().unwrap();
    d.clear_history().unwrap();
    assert_eq!(snap(&d), before);
    assert_eq!(d.move_mask(a, a), Ok(()), "同じレイヤーへは何もしない");
    assert_eq!(snap(&d), before);
    let stroke = d
        .begin_stroke(b, &yolu_core::BrushSettings::default())
        .unwrap();
    assert_eq!(d.move_mask(a, b), Err(CoreError::StrokeActive));
    assert_eq!(d.add_black_layer_mask(b), Err(CoreError::StrokeActive));
    assert_eq!(d.copy_mask(a).unwrap_err(), CoreError::StrokeActive);
    let copy = {
        d.cancel_stroke(stroke);
        d.copy_mask(a).unwrap()
    };
    let stroke = d
        .begin_stroke(b, &yolu_core::BrushSettings::default())
        .unwrap();
    assert_eq!(d.set_masks(&[b], &copy), Err(CoreError::StrokeActive));
    d.cancel_stroke(stroke);
}

#[test]
fn a_generator_reading_the_mask_anchor_keeps_reading_it_after_the_mask_moves() {
    let mut d = doc();
    let a = full(&mut d, "a", Rgba8::new(255, 0, 0, 255));
    let b = full(&mut d, "b", Rgba8::new(0, 255, 0, 255));
    let c = full(&mut d, "c", Rgba8::new(0, 0, 255, 255));
    d.add_layer_mask(a).unwrap();
    let anchor = d.add_anchor(a, AnchorPlacement::Mask, None, None).unwrap();
    let mut g = Settings::new(generator::Kind::Anchor);
    g.blend = generator::Blend::Replace;
    let reader = d
        .add_filter(
            c,
            FilterTarget::Content,
            FilterSpec::new(EffectSettings::generator(g)).channels(&[Channel::Height]),
        )
        .unwrap();
    d.set_generator_anchor(
        c,
        reader,
        Some(anchor),
        Channel::Height,
        ReadMode::Value,
        false,
    )
    .unwrap();
    assert!(d.anchor_issues().is_empty());
    d.move_mask(a, b).unwrap();
    assert!(
        d.anchor_issues().is_empty(),
        "読む側より下のレイヤーなら切れない"
    );
    assert_eq!(d.find_anchor(anchor).unwrap().layer, b);
    // 読む側より上（ここでは読む側自身）へ動かすと、黙って切れずに問題として挙がる。取り消しで戻る
    d.move_mask(b, c).unwrap();
    assert_eq!(d.anchor_issues().len(), 1);
    assert_eq!(d.anchor_issues()[0].kind, AnchorIssueKind::NotBelow);
    d.undo().unwrap();
    assert!(d.anchor_issues().is_empty());
}

// ───────── マスクへの画像の貼り付け ─────────

fn grey_image(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 4]) -> PixelClipboard {
    let mut rgba = Vec::new();
    for y in 0..h {
        for x in 0..w {
            rgba.extend_from_slice(&f(x, y));
        }
    }
    PixelClipboard::from_image(w, h, rgba, Channel::Color).unwrap()
}

#[test]
fn pasting_an_image_into_a_mask_reads_white_as_visible_in_one_undo_step() {
    let (mut d, a, _b) = two();
    d.add_layer_mask(a).unwrap();
    d.clear_history().unwrap();
    // 左半分が白（見える）、右半分が黒（隠す）の、キャンバスと同じ大きさの画像
    let clip = grey_image(W, H, |x, _| {
        if x < 20 {
            [255, 255, 255, 255]
        } else {
            [0, 0, 0, 255]
        }
    });
    let mut result = None;
    one_undo_step(&mut d, "画像を貼る", |d| {
        result = Some(d.paste_into_mask(a, &clip).unwrap())
    });
    let result = result.unwrap();
    assert!(result.changed && !result.centered);
    assert_eq!(result.clipped_pixels, 0);
    assert_eq!(last_kind(&d), HistoryKind::PasteMask);
    assert_eq!(hide_at(&d, a, 3, 3), 0, "白は見える");
    assert_eq!(hide_at(&d, a, 30, 3), 255, "黒は隠す");
}

#[test]
fn a_coloured_image_becomes_its_brightness_and_the_copy_round_trips() {
    let (mut d, a, b) = two();
    d.add_layer_mask(a).unwrap();
    d.fill_mask(
        a,
        0.5,
        Some(&SelectionMask::rectangle(&d, 4, 4, 30, 30)),
        false,
    )
    .unwrap();
    d.add_layer_mask(b).unwrap();
    d.clear_history().unwrap();
    // マスクのコピー（灰色）をそのまま別のマスクへ貼ると、同じ画素になる
    let clip = d.copy_pixels(a, Channel::Color, true, u64::MAX).unwrap();
    d.paste_into_mask(b, &clip).unwrap();
    assert_eq!(
        d.layer(a)
            .unwrap()
            .mask()
            .unwrap()
            .surface()
            .canvas_bytes()
            .unwrap(),
        d.layer(b)
            .unwrap()
            .mask()
            .unwrap()
            .surface()
            .canvas_bytes()
            .unwrap(),
    );
    // 色のある画像: 純色の赤は Rec.709 で 0.2126 → 見える量 54、隠す量 201
    let red = grey_image(W, H, |_, _| [255, 0, 0, 255]);
    d.paste_into_mask(b, &red).unwrap();
    assert_eq!(hide_at(&d, b, 1, 1), 255 - 54);
}

#[test]
fn transparent_pixels_of_the_image_leave_the_mask_and_half_transparent_ones_mix() {
    let (mut d, a, _b) = two();
    d.add_layer_mask(a).unwrap();
    d.fill_mask(a, 1.0, None, false).unwrap(); // 全部隠す
    d.clear_history().unwrap();
    let clip = grey_image(W, H, |x, _| match x {
        0..=9 => [255, 255, 255, 255],
        10..=19 => [255, 255, 255, 0],
        _ => [255, 255, 255, 128],
    });
    d.paste_into_mask(a, &clip).unwrap();
    assert_eq!(hide_at(&d, a, 2, 2), 0);
    assert_eq!(hide_at(&d, a, 12, 2), 255, "透明はそのまま");
    let mixed = hide_at(&d, a, 30, 2);
    assert!(
        (125..=130).contains(&mixed),
        "半透明は今の値（隠す）と混ぜる: {mixed}"
    );
}

#[test]
fn a_smaller_image_from_a_different_canvas_is_centred_and_clipped() {
    let (mut d, a, _b) = two();
    d.add_layer_mask(a).unwrap();
    d.clear_history().unwrap();
    // 画像の大きさが違う文書へは中央（小さければ中央に置く）
    let small = grey_image(11, 7, |_, _| [0, 0, 0, 255]);
    let r = d.paste_into_mask(a, &small).unwrap();
    assert!(r.centered);
    assert_eq!(r.clipped_pixels, 0);
    let (ox, oy) = ((W - 11) / 2, (H - 7) / 2);
    assert_eq!(hide_at(&d, a, ox, oy), 255);
    assert_eq!(hide_at(&d, a, ox + 10, oy + 6), 255);
    assert_eq!(hide_at(&d, a, ox - 1, oy), 0, "外は変わらない");
    // 大きい画像ははみ出す分を切って数える
    d.undo().unwrap();
    let big = grey_image(W + 4, H, |_, _| [0, 0, 0, 255]);
    let r = d.paste_into_mask(a, &big).unwrap();
    assert!(r.centered);
    assert_eq!(r.clipped_pixels, 4 * H as u64);
}

#[test]
fn pasting_into_a_mask_is_refused_without_a_mask_and_on_a_lock() {
    let (mut d, a, _b) = two();
    let clip = grey_image(W, H, |_, _| [0, 0, 0, 255]);
    assert_eq!(
        d.paste_into_mask(a, &clip),
        Err(CoreError::Unsupported(Reason::LayerHasNoMask))
    );
    d.add_layer_mask(a).unwrap();
    d.set_layer_locks(a, LayerLocks::ALL).unwrap();
    d.clear_history().unwrap();
    let before = snap(&d);
    assert!(matches!(
        d.paste_into_mask(a, &clip),
        Err(CoreError::LayerLocked { .. })
    ));
    assert_eq!(snap(&d), before);
}

#[test]
fn pasting_an_unchanged_image_pushes_no_step() {
    let (mut d, a, _b) = two();
    d.add_layer_mask(a).unwrap();
    d.clear_history().unwrap();
    let white = grey_image(W, H, |_, _| [255, 255, 255, 255]);
    let r = d.paste_into_mask(a, &white).unwrap();
    assert!(!r.changed, "もう何も隠していない");
    assert_eq!(d.undo_count(), 0);
}
