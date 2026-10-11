//! レイヤーの画素のコピー・カット・結合してコピー・ペースト・画像での置き換えと、複数の編集を 1 回の Undo にまとめる `batch`。
//! C# の LayerOpsTests のコピー系 7 件と DocumentBatchTests の 7 件の確かめを移し、ロック・チャンネル・予算・取消・保存前後の
//! 一致・まとめの中の禁止を足したもの（実 C# Core との全バイトの照合は clipboard_golden.rs）。

// 画素の格子を (x, y) の添字で見比べる試験なので、添字の範囲の繰り返しの方が読みやすい。
#![allow(clippy::needless_range_loop)]

use std::panic::{catch_unwind, AssertUnwindSafe};

use yolu_core::glam::DVec2;
use yolu_core::material::ChannelPaint;
use yolu_core::{
    AdjustmentSettings, BlendMode, Brush, BrushSettings, Channel, ChannelInfo, ChannelKind,
    ClipboardRefusal, ClipboardSource, ColorSpace, CoreError, Document, LayerId, LayerLocks,
    PixelClipboard, Rgba8, SelectionMask, TileCoord,
};

const W: u32 = 41;
const H: u32 = 35;
const TILE: u32 = 16;

fn doc(w: u32, h: u32) -> Document {
    Document::with_tile_size(w, h, TILE).unwrap()
}
fn rng(seed: &mut u64) -> u8 {
    *seed = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    (*seed >> 33) as u8
}
fn random_color(seed: &mut u64) -> Rgba8 {
    Rgba8::new(rng(seed), rng(seed), rng(seed), rng(seed))
}
/// 面へ散らす（透明だが RGB のある画素・不透明な画素・半透明の画素）。
fn scatter(d: &mut Document, layer: LayerId, channel: Channel, seed: &mut u64, count: usize) {
    for _ in 0..count {
        let c = random_color(seed);
        let kind = rng(seed) % 3;
        let color = match kind {
            0 => Rgba8::new(c.r, c.g, c.b, 0),
            1 => Rgba8::new(c.r, c.g, c.b, 255),
            _ => c,
        };
        let (x, y) = (rng(seed) as u32 % d.width(), rng(seed) as u32 % d.height());
        d.set_channel_pixel(layer, channel, x, y, color).unwrap();
    }
}
fn px(d: &Document, layer: LayerId, channel: Channel, x: u32, y: u32) -> Rgba8 {
    d.layer(layer).unwrap().pixel(channel, x, y).unwrap()
}
fn layer_bytes(d: &Document, layer: LayerId, channel: Channel) -> Vec<u8> {
    let l = d.layer(layer).unwrap();
    (0..d.height())
        .flat_map(|y| (0..d.width()).map(move |x| (x, y)))
        .flat_map(|(x, y)| l.pixel(channel, x, y).unwrap().to_array())
        .collect()
}
fn refusal(result: Result<impl std::fmt::Debug, CoreError>) -> ClipboardRefusal {
    match result {
        Err(CoreError::Clipboard(r)) => r,
        other => panic!("断られるはず: {other:?}"),
    }
}

/// レイヤーの 1 行（ID・名前・親・表示・不透明度の bit・合成モード・ロック）。
type LayerRow = (LayerId, String, Option<LayerId>, bool, u64, BlendMode, u64);

/// 文書の見える状態の全部（履歴・変更番号・レイヤーの並びと属性・全チャンネルの画素・マスク・選択範囲・合成）。断った操作の前後で
/// 変わってはいけないもの。
#[derive(PartialEq)]
struct State {
    revision: u64,
    undo: usize,
    redo: usize,
    history: u64,
    layers: Vec<LayerRow>,
    surfaces: Vec<(LayerId, Channel, bool, Vec<u8>)>,
    masks: Vec<(LayerId, Vec<u8>)>,
    selection: Option<Vec<u8>>,
    composites: Vec<Vec<u8>>,
}
/// 画素が大きいので、食い違ったときは項目ごとの数と指紋だけを出す。
impl std::fmt::Debug for State {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use std::hash::{Hash, Hasher};
        let fingerprint = |bytes: &mut dyn Iterator<Item = &Vec<u8>>| {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            bytes.for_each(|b| b.hash(&mut h));
            h.finish()
        };
        write!(
            f,
            "State {{ revision: {}, undo: {}, redo: {}, history: {}, layers: {:?}, surfaces: {} 面 {:016x}, masks: {} 枚 {:016x}, selection: {:?}, composites: {:016x} }}",
            self.revision,
            self.undo,
            self.redo,
            self.history,
            self.layers,
            self.surfaces.len(),
            fingerprint(&mut self.surfaces.iter().map(|s| &s.3)),
            self.masks.len(),
            fingerprint(&mut self.masks.iter().map(|m| &m.1)),
            self.selection.as_ref().map(|s| fingerprint(&mut std::iter::once(s))),
            fingerprint(&mut self.composites.iter()),
        )
    }
}
fn state(d: &Document) -> State {
    let mut surfaces = Vec::new();
    let mut masks = Vec::new();
    for l in d.layers() {
        for c in d.channels() {
            surfaces.push((
                l.id(),
                c,
                l.is_channel_enabled(c),
                match l.surface(c) {
                    Some(s) => s.to_canvas_bytes(),
                    None => Vec::new(),
                },
            ));
        }
        if let Some(m) = l.mask() {
            masks.push((l.id(), m.surface().to_canvas_bytes()));
        }
    }
    State {
        revision: d.revision(),
        undo: d.undo_count(),
        redo: d.redo_count(),
        history: d.history_bytes(),
        layers: d
            .layers()
            .iter()
            .map(|l| {
                (
                    l.id(),
                    l.name().to_string(),
                    l.parent(),
                    l.visible(),
                    l.opacity().to_bits(),
                    l.blend_mode(),
                    l.locks().bits() as u64,
                )
            })
            .collect(),
        surfaces,
        masks,
        selection: d.selection().map(|s| s.to_canvas_bytes()),
        composites: d
            .channels()
            .into_iter()
            .map(|c| d.composite_channel(c, d.bounds()).unwrap())
            .collect(),
    }
}
/// `state` から変更番号だけを除く（Undo・Redo で戻ったあとは、中身が同じで変更番号は進んでいる）。
fn content(d: &Document) -> State {
    State {
        revision: 0,
        ..state(d)
    }
}

/// 履歴と変更番号を除いた、文書の中身（Undo・Redo で戻ったあとの比べ用。戻した段は Redo に残るので履歴の数とバイトは違う）。
fn shape(d: &Document) -> State {
    State {
        revision: 0,
        undo: 0,
        redo: 0,
        history: 0,
        ..state(d)
    }
}

fn image(w: u32, h: u32, pixel: impl Fn(u32, u32) -> Rgba8) -> Vec<u8> {
    (0..h)
        .flat_map(|y| (0..w).map(move |x| (x, y)))
        .flat_map(|(x, y)| pixel(x, y).to_array())
        .collect()
}
const MAX: u64 = u64::MAX;

// ───────── コピー ─────────

#[test]
fn copy_takes_the_selection_with_premultiplied_partial_amounts_and_keeps_transparent_rgb() {
    let mut d = doc(W, H);
    let l = d.add_layer("l").unwrap();
    let hidden = Rgba8::new(200, 100, 50, 0);
    let half = Rgba8::new(10, 220, 30, 128);
    let solid = Rgba8::new(1, 2, 3, 255);
    d.set_pixel(l, 10, 10, hidden).unwrap();
    d.set_pixel(l, 11, 10, half).unwrap();
    d.set_pixel(l, 12, 10, solid).unwrap();
    d.set_pixel(l, 30, 30, solid).unwrap();
    // 10..12 の 3 画素だけ全部入る
    let square =
        [(9.0, 9.0), (13.0, 9.0), (13.0, 11.0), (9.0, 11.0)].map(|(x, y)| DVec2::new(x, y));
    d.set_selection(Some(SelectionMask::polygon(&d, &square).unwrap()))
        .unwrap();
    let copied = d.copy_pixels(l, Channel::Color, false, MAX).unwrap();
    let r = copied.rect();
    assert_eq!(
        (r.x, r.y, r.width, r.height),
        (10, 10, 3, 1),
        "値のある画素まで切り詰める"
    );
    assert_eq!(
        copied.pixel(0, 0).unwrap(),
        hidden,
        "全部選ばれた透明画素は RGB ごと"
    );
    assert_eq!(copied.pixel(1, 0).unwrap(), half);
    assert_eq!(copied.pixel(2, 0).unwrap(), solid);
    assert_eq!(copied.source(), ClipboardSource::Layer);
    assert_eq!(copied.channel(), Channel::Color);
    assert_eq!(copied.document_size(), (W, H));

    // 部分的な選択: 透明へ向かってプリマルチプライドで混ぜる（色は保ち、アルファが減る）
    d.set_selection(Some(
        SelectionMask::ellipse(&d, 12.5, 10.5, 1.2, 1.2).unwrap(),
    ))
    .unwrap();
    let faded = d.copy_pixels(l, Channel::Color, false, MAX).unwrap();
    assert!(faded.width() <= 3);
    let sel = d.selection().unwrap().clone();
    for y in 0..faded.height() {
        for x in 0..faded.width() {
            let (gx, gy) = (faded.rect().x + x, faded.rect().y + y);
            let amount = sel.amount(gx, gy);
            let p = px(&d, l, Channel::Color, gx, gy);
            let got = faded.pixel(x, y).unwrap();
            if amount == 255 {
                assert_eq!(got, p);
            } else if amount == 0 || p.a == 0 {
                assert_eq!(got, Rgba8::TRANSPARENT);
            } else {
                assert_eq!((got.r, got.g, got.b), (p.r, p.g, p.b));
                let expected = (p.a as f64 * amount as f64 / 255.0 + 0.5).floor();
                assert!(
                    (got.a as f64 - expected).abs() <= 1.0,
                    "{got:?} {p:?} {amount}"
                );
            }
        }
    }
}

#[test]
fn copy_merged_takes_the_composite_inside_the_selection() {
    let mut seed = 6;
    let mut d = doc(W, H);
    let a = d.add_layer("a").unwrap();
    scatter(&mut d, a, Channel::Color, &mut seed, 500);
    let b = d.add_layer("b").unwrap();
    scatter(&mut d, b, Channel::Color, &mut seed, 500);
    d.set_layer_blend_mode(b, BlendMode::Screen).unwrap();
    d.set_layer_opacity(b, 0.6, false).unwrap();
    let copied = d.copy_merged(Channel::Color, MAX).unwrap();
    let composite = d.composite(d.bounds()).unwrap();
    let r = copied.rect();
    for y in 0..copied.height() {
        for x in 0..copied.width() {
            let o = (((r.y + y) * W + r.x + x) * 4) as usize;
            assert_eq!(
                copied.pixel(x, y).unwrap().to_array(),
                composite[o..o + 4],
                "({x}, {y})"
            );
        }
    }
    assert_eq!(copied.source(), ClipboardSource::Composite);

    // 選択範囲の中だけ（外は 0 で、矩形はその中に収まる）
    d.set_selection(Some(SelectionMask::rectangle(&d, 5, 6, 20, 12)))
        .unwrap();
    let inside = d.copy_merged(Channel::Color, MAX).unwrap();
    let r = inside.rect();
    assert!(
        r.x >= 5 && r.y >= 6 && r.x + r.width <= 20 && r.y + r.height <= 12,
        "{r:?}"
    );
    for y in 0..inside.height() {
        for x in 0..inside.width() {
            let o = (((r.y + y) * W + r.x + x) * 4) as usize;
            assert_eq!(inside.pixel(x, y).unwrap().to_array(), composite[o..o + 4]);
        }
    }
}

#[test]
fn a_mask_copies_as_grey_and_cutting_it_reveals() {
    let mut d = doc(W, H);
    let l = d.add_layer("l").unwrap();
    d.set_pixel(l, 3, 3, Rgba8::new(9, 9, 9, 255)).unwrap();
    d.add_layer_mask(l).unwrap();
    d.set_mask_pixel(l, 5, 5, 200).unwrap();
    d.set_selection(Some(SelectionMask::rectangle(&d, 4, 4, 8, 8)))
        .unwrap();
    d.clear_history().unwrap();
    let copied = d.copy_pixels(l, Channel::Color, true, MAX).unwrap();
    assert_eq!(copied.source(), ClipboardSource::Mask);
    let r = copied.rect();
    assert_eq!(
        (r.x, r.y, r.width, r.height),
        (4, 4, 4, 4),
        "塗っていないマスクは白なので、選択範囲の全部を写す"
    );
    assert_eq!(
        copied.pixel(1, 1).unwrap(),
        Rgba8::new(55, 55, 55, 255),
        "隠す量 200 は灰色 55"
    );
    assert_eq!(copied.pixel(0, 0).unwrap(), Rgba8::new(255, 255, 255, 255));

    let cut = d.cut_pixels(l, Channel::Color, true, MAX).unwrap();
    assert_eq!(cut.pixels(), copied.pixels());
    let mask = d.layer(l).unwrap().mask().unwrap();
    assert_eq!(
        mask.surface().pixel(5, 5).unwrap().a,
        0,
        "マスクを切ると見える"
    );
    assert_eq!(
        px(&d, l, Channel::Color, 3, 3).a,
        255,
        "レイヤーの画素はそのまま"
    );
    assert_eq!(d.undo_count(), 1);
    d.undo().unwrap();
    assert_eq!(
        d.layer(l)
            .unwrap()
            .mask()
            .unwrap()
            .surface()
            .pixel(5, 5)
            .unwrap()
            .a,
        200
    );
}

#[test]
fn cut_copies_and_erases_the_selection_as_one_undo_step() {
    let mut d = doc(W, H);
    let l = d.add_layer("l").unwrap();
    for y in 0..20 {
        for x in 0..20 {
            d.set_pixel(l, x, y, Rgba8::new(100, 150, 200, 255))
                .unwrap();
        }
    }
    d.set_pixel(l, 2, 2, Rgba8::new(50, 60, 70, 0)).unwrap();
    d.set_selection(Some(SelectionMask::rectangle(&d, 0, 0, 10, 10)))
        .unwrap();
    d.clear_history().unwrap();
    let before = shape(&d);
    let cut = d.cut_pixels(l, Channel::Color, false, MAX).unwrap();
    let r = cut.rect();
    assert_eq!((r.x, r.y, r.width, r.height), (0, 0, 10, 10));
    assert_eq!(
        cut.pixel(2, 2).unwrap(),
        Rgba8::new(50, 60, 70, 0),
        "カットも透明画素の RGB を写す"
    );
    assert_eq!(
        px(&d, l, Channel::Color, 5, 5),
        Rgba8::TRANSPARENT,
        "RGB ごと消える"
    );
    assert_eq!(
        px(&d, l, Channel::Color, 15, 15).a,
        255,
        "選択の外はそのまま"
    );
    assert_eq!(d.undo_count(), 1);
    assert!(d.undo().unwrap());
    assert_eq!(shape(&d), before);
    assert!(d.redo().unwrap());
    assert_eq!(px(&d, l, Channel::Color, 5, 5), Rgba8::TRANSPARENT);
}

#[test]
fn a_partially_selected_cut_fades_alpha_and_copies_the_faded_pixels() {
    let mut d = doc(W, H);
    let l = d.add_layer("l").unwrap();
    for y in 8..14 {
        for x in 8..14 {
            d.set_pixel(l, x, y, Rgba8::new(90, 120, 30, 200)).unwrap();
        }
    }
    d.set_selection(Some(
        SelectionMask::ellipse(&d, 10.5, 10.5, 2.4, 2.4).unwrap(),
    ))
    .unwrap();
    let sel = d.selection().unwrap().clone();
    let cut = d.cut_pixels(l, Channel::Color, false, MAX).unwrap();
    let mut partial = 0;
    for y in 8..14 {
        for x in 8..14 {
            let amount = sel.amount(x, y);
            let left = px(&d, l, Channel::Color, x, y);
            match amount {
                255 => assert_eq!(left, Rgba8::TRANSPARENT),
                0 => assert_eq!(left, Rgba8::new(90, 120, 30, 200)),
                _ => {
                    partial += 1;
                    assert_eq!(
                        (left.r, left.g, left.b),
                        (90, 120, 30),
                        "部分的な量は色を保つ"
                    );
                    assert!(left.a < 200 && left.a > 0, "{left:?}");
                    // 写したものと残ったものの重なりは元の画素の量
                    let (cx, cy) = (x - cut.rect().x, y - cut.rect().y);
                    assert!(cut.pixel(cx, cy).unwrap().a > 0);
                }
            }
        }
    }
    assert!(partial > 0, "縁に部分的な画素がある");
}

#[test]
fn copy_cut_and_paste_refuse_with_a_reason_and_change_nothing() {
    let mut d = doc(W, H);
    let empty = d.add_layer("empty").unwrap();
    let fill = d
        .add_fill_layer("fill", &[(Channel::Color, Rgba8::new(1, 2, 3, 255))], None)
        .unwrap();
    let adjust = d
        .add_adjustment_layer("adjust", AdjustmentSettings::invert(), None, None)
        .unwrap();
    let group = d.add_group("group", None).unwrap();
    let painted = d.add_layer("painted").unwrap();
    d.set_pixel(painted, 5, 5, Rgba8::new(9, 9, 9, 255))
        .unwrap();
    d.set_pixel(painted, 20, 5, Rgba8::new(9, 9, 9, 255))
        .unwrap();
    d.clear_history().unwrap();
    let before = state(&d);
    assert_eq!(
        refusal(d.copy_pixels(empty, Channel::Color, false, MAX)),
        ClipboardRefusal::NothingToCopy { selection: false }
    );
    assert_eq!(
        refusal(d.copy_pixels(adjust, Channel::Color, false, MAX)),
        ClipboardRefusal::NoPixels
    );
    assert_eq!(
        refusal(d.copy_pixels(group, Channel::Color, false, MAX)),
        ClipboardRefusal::NoPixels
    );
    assert_eq!(
        refusal(d.cut_pixels(fill, Channel::Color, false, MAX)),
        ClipboardRefusal::NotPaintLayer
    );
    assert_eq!(
        refusal(d.cut_pixels(group, Channel::Color, false, MAX)),
        ClipboardRefusal::NoPixels
    );
    // 上限: 最初のタイルを足した時点の矩形（16 × 16 × 4）で断る
    assert_eq!(
        refusal(d.copy_pixels(fill, Channel::Color, false, 1000)),
        ClipboardRefusal::TooLarge {
            bytes: 16 * 16 * 4,
            limit: 1000
        }
    );
    assert_eq!(
        d.copy_pixels(fill, Channel::Color, false, MAX)
            .unwrap()
            .width(),
        W,
        "塗りつぶしレイヤーは値をキャンバスの全体へ写す"
    );
    assert_eq!(
        d.copy_pixels(empty, Channel::Color, true, MAX),
        Err(CoreError::Unsupported("レイヤーにマスクが無い"))
    );
    assert_eq!(
        d.cut_pixels(empty, Channel::Roughness, false, MAX),
        Err(CoreError::Unsupported("無効のチャンネルは切り取れない"))
    );
    assert_eq!(
        d.copy_pixels(painted, Channel::from_index(40).unwrap(), false, MAX),
        Err(CoreError::ChannelNotFound)
    );

    // 貼るレイヤーが一操作の予算を超える
    let copy = d.copy_pixels(painted, Channel::Color, false, MAX).unwrap();
    d.set_stroke_budget_bytes(100).unwrap();
    assert!(matches!(
        refusal(d.paste_as_layer(&copy, Channel::Color, None, None)),
        ClipboardRefusal::OperationBudget { limit: 100, .. }
    ));
    d.set_stroke_budget_bytes(64 << 20).unwrap();
    // 画素の予算を超える（タイルを足せない）
    let budget = d.source_budget_bytes();
    d.set_source_budget_bytes(d.allocated_bytes() + 8).unwrap();
    assert_eq!(
        d.paste_as_layer(&copy, Channel::Color, None, None),
        Err(CoreError::SourceBudgetExceeded)
    );
    d.set_source_budget_bytes(budget).unwrap();
    assert_eq!(state(&d), before, "何も変わらず、何も記録しない");
    assert!(!d.can_undo());
    assert_eq!(
        d.paste_as_layer(&copy, Channel::from_index(40).unwrap(), None, None),
        Err(CoreError::ChannelNotFound)
    );
    assert_eq!(
        d.paste_as_layer(&copy, Channel::Color, None, Some(LayerId(77))),
        Err(CoreError::LayerNotFound)
    );
    assert_eq!(state(&d), before);

    // ストロークの途中は、コピーもカットもペーストも置き換えも断る
    let stroke = d.begin_stroke(empty, &BrushSettings::default()).unwrap();
    assert_eq!(
        d.copy_pixels(painted, Channel::Color, false, MAX),
        Err(CoreError::StrokeActive)
    );
    assert_eq!(
        d.copy_merged(Channel::Color, MAX),
        Err(CoreError::StrokeActive)
    );
    assert_eq!(
        d.cut_pixels(painted, Channel::Color, false, MAX),
        Err(CoreError::StrokeActive)
    );
    assert_eq!(
        d.paste_as_layer(&copy, Channel::Color, None, None),
        Err(CoreError::StrokeActive)
    );
    assert_eq!(
        d.replace_pixels(
            painted,
            Channel::Color,
            &vec![0; (W * H * 4) as usize],
            true
        ),
        Err(CoreError::StrokeActive)
    );
    d.cancel_stroke(stroke);

    // 写しの作り方の検査
    let ch = Channel::Color;
    let layer = ClipboardSource::Layer;
    assert!(PixelClipboard::new(10, 10, 5, 5, 6, 1, vec![0; 24], layer, ch).is_err());
    assert!(PixelClipboard::new(10, 10, 0, 0, 2, 2, vec![0; 15], layer, ch).is_err());
    assert!(PixelClipboard::new(10, 10, 0, 0, 0, 2, vec![], layer, ch).is_err());
    assert!(PixelClipboard::new(0, 10, 0, 0, 1, 1, vec![0; 4], layer, ch).is_err());
    assert!(PixelClipboard::new(10, 10, 9, 9, 1, 1, vec![0; 4], layer, ch).is_ok());
}

#[test]
fn nothing_inside_the_selection_is_refused_with_the_selection_named() {
    let mut d = doc(W, H);
    let l = d.add_layer("l").unwrap();
    d.set_pixel(l, 30, 30, Rgba8::new(1, 1, 1, 255)).unwrap();
    d.set_selection(Some(SelectionMask::rectangle(&d, 0, 0, 5, 5)))
        .unwrap();
    d.clear_history().unwrap();
    let before = state(&d);
    assert_eq!(
        refusal(d.copy_pixels(l, Channel::Color, false, MAX)),
        ClipboardRefusal::NothingToCopy { selection: true }
    );
    assert_eq!(
        refusal(d.cut_pixels(l, Channel::Color, false, MAX)),
        ClipboardRefusal::NothingToCopy { selection: true }
    );
    assert_eq!(
        refusal(d.copy_merged(Channel::Color, MAX)),
        ClipboardRefusal::NothingToCopy { selection: true }
    );
    assert_eq!(state(&d), before);
}

// ───────── ペースト ─────────

#[test]
fn paste_makes_a_new_layer_in_place_and_one_undo_takes_it_away() {
    let mut seed = 4;
    let mut d = doc(W, H);
    let l = d.add_layer("l").unwrap();
    scatter(&mut d, l, Channel::Roughness, &mut seed, 400);
    d.set_channel_enabled(l, Channel::Roughness, true).unwrap();
    let copied = d.copy_pixels(l, Channel::Roughness, false, MAX).unwrap();
    let sel = SelectionMask::rectangle(&d, 0, 0, 5, 5);
    d.set_selection(Some(sel.clone())).unwrap();
    d.clear_history().unwrap();
    let before = shape(&d);

    let pasted = d
        .paste_as_layer(&copied, Channel::Roughness, Some("pasted"), Some(l))
        .unwrap();
    assert!(!pasted.centered);
    assert_eq!(pasted.clipped_pixels, 0);
    assert_eq!(d.layers().last().unwrap().id(), pasted.layer);
    assert_eq!(d.layer(pasted.layer).unwrap().name(), "pasted");
    assert_eq!(
        layer_bytes(&d, pasted.layer, Channel::Roughness),
        layer_bytes(&d, l, Channel::Roughness),
        "同じ位置に同じ画素（透明画素の RGB も）"
    );
    assert_eq!(
        d.layer(pasted.layer).unwrap().surface_channels(),
        vec![Channel::Roughness],
        "指定したチャンネルだけを持つ"
    );
    assert!(d
        .layer(pasted.layer)
        .unwrap()
        .is_channel_enabled(Channel::Roughness));
    assert!(
        d.selection().is_none(),
        "貼ると選択を外す（そのまま動かせる）"
    );
    assert_eq!(
        d.undo_count(),
        1,
        "レイヤーを足すことと選択を外すことは 1 回の Undo"
    );
    assert!(d.undo().unwrap());
    assert_eq!(shape(&d), before);
    assert!(d.selection().unwrap().same_as(&sel), "Undo で選択も戻る");
    assert!(d.redo().unwrap());
    assert_eq!(d.layers().len(), 2);
    assert!(d.selection().is_none());
    // 貼ったレイヤーはふつうのレイヤー: 合成にも出て、全チャンネルの合成が参照の式と同じ
    assert_eq!(
        d.composite_channel(Channel::Roughness, d.bounds())
            .unwrap()
            .len(),
        (W * H * 4) as usize
    );
}

#[test]
fn pasting_without_a_selection_is_a_single_plain_step() {
    let mut d = doc(W, H);
    let l = d.add_layer("l").unwrap();
    d.set_pixel(l, 4, 4, Rgba8::new(7, 8, 9, 255)).unwrap();
    let copied = d.copy_pixels(l, Channel::Color, false, MAX).unwrap();
    d.clear_history().unwrap();
    let pasted = d
        .paste_as_layer(&copied, Channel::Color, None, None)
        .unwrap();
    assert_eq!(d.undo_count(), 1);
    assert_eq!(d.layer(pasted.layer).unwrap().name(), "Layer");
    assert_eq!(
        px(&d, pasted.layer, Channel::Color, 4, 4),
        Rgba8::new(7, 8, 9, 255)
    );
    assert!(d.undo().unwrap());
    assert_eq!(d.layers().len(), 1);
}

#[test]
fn pasting_above_a_layer_in_a_group_goes_into_that_group_and_ignores_other_locks() {
    let mut d = doc(W, H);
    let a = d.add_layer("a").unwrap();
    let b = d.add_layer("b").unwrap();
    d.set_pixel(a, 1, 1, Rgba8::new(5, 5, 5, 255)).unwrap();
    let group = d.group_layers(&[a, b], "g").unwrap();
    d.set_layer_locks(group, LayerLocks::ALL).unwrap();
    let copied = d.copy_pixels(a, Channel::Color, false, MAX).unwrap();
    let pasted = d
        .paste_as_layer(&copied, Channel::Color, None, Some(a))
        .unwrap();
    let l = d.layer(pasted.layer).unwrap();
    assert_eq!(l.parent(), Some(group), "above と同じグループの中");
    let pos = |id: LayerId| d.layer_index(id).unwrap();
    assert_eq!(pos(pasted.layer), pos(a) + 1);
}

#[test]
fn pasting_into_a_document_of_another_size_centers_and_cuts_off_what_falls_outside() {
    let mut big = doc(64, 48);
    let l = big.add_layer("l").unwrap();
    for y in 10..40 {
        for x in 2..62 {
            big.set_pixel(l, x, y, Rgba8::new(x as u8, y as u8, 7, (x * 4) as u8))
                .unwrap();
        }
    }
    let copied = big.copy_pixels(l, Channel::Color, false, MAX).unwrap();
    let r = copied.rect();
    assert_eq!((r.x, r.y, r.width, r.height), (2, 10, 60, 30));

    let mut small = doc(40, 40);
    small.add_layer("base").unwrap();
    let pasted = small
        .paste_as_layer(&copied, Channel::Color, None, None)
        .unwrap();
    assert!(pasted.centered);
    assert_eq!((pasted.x, pasted.y), ((40 - 60) / 2, (40 - 30) / 2));
    let mut expected = 0u64;
    for y in 0..30 {
        for x in 0..60 {
            let px = pasted.x + x as i32;
            if !(0..40).contains(&px) && copied.pixel(x, y).unwrap() != Rgba8::TRANSPARENT {
                expected += 1;
            }
        }
    }
    assert_eq!(pasted.clipped_pixels, expected);
    assert!(expected > 0);
    assert_eq!(
        px(&small, pasted.layer, Channel::Color, 0, 5),
        copied.pixel((-pasted.x) as u32, 0).unwrap()
    );
    assert_eq!(
        px(&small, pasted.layer, Channel::Color, 39, 34),
        copied.pixel((39 - pasted.x) as u32, 29).unwrap()
    );
}

#[test]
fn an_odd_centering_rounds_towards_zero_like_the_original() {
    // 幅の差が奇数のとき、正は切り捨て・負は 0 へ丸める（C# の整数の割り算）
    let mut a = doc(7, 7);
    let l = a.add_layer("l").unwrap();
    for y in 0..7 {
        for x in 0..7 {
            a.set_pixel(l, x, y, Rgba8::new(x as u8, y as u8, 1, 255))
                .unwrap();
        }
    }
    let copied = a.copy_pixels(l, Channel::Color, false, MAX).unwrap();
    let mut wide = doc(10, 10);
    wide.add_layer("base").unwrap();
    let p = wide
        .paste_as_layer(&copied, Channel::Color, None, None)
        .unwrap();
    assert_eq!((p.x, p.y, p.clipped_pixels), (1, 1, 0), "(10 − 7) / 2 = 1");
    let mut narrow = doc(4, 4);
    narrow.add_layer("base").unwrap();
    let p = narrow
        .paste_as_layer(&copied, Channel::Color, None, None)
        .unwrap();
    assert_eq!((p.x, p.y), (-1, -1), "(4 − 7) / 2 = −1（0 へ丸める）");
    assert_eq!(p.clipped_pixels, 49 - 16);
}

#[test]
fn an_external_image_is_centred_unless_it_has_the_documents_size() {
    let rgba = image(6, 4, |x, y| Rgba8::new(x as u8 * 10, y as u8 * 10, 3, 255));
    let clip = PixelClipboard::from_image(6, 4, rgba.clone(), Channel::Color).unwrap();
    assert_eq!(clip.source(), ClipboardSource::External);
    let mut d = doc(20, 12);
    d.add_layer("base").unwrap();
    let p = d.paste_as_layer(&clip, Channel::Color, None, None).unwrap();
    assert!(p.centered);
    assert_eq!((p.x, p.y, p.clipped_pixels), (7, 4, 0));
    assert_eq!(
        px(&d, p.layer, Channel::Color, 7, 4),
        Rgba8::new(0, 0, 3, 255)
    );
    assert_eq!(
        px(&d, p.layer, Channel::Color, 12, 7),
        Rgba8::new(50, 30, 3, 255)
    );
    assert_eq!(px(&d, p.layer, Channel::Color, 6, 4), Rgba8::TRANSPARENT);
    let mut same = doc(6, 4);
    same.add_layer("base").unwrap();
    let p = same
        .paste_as_layer(&clip, Channel::Color, None, None)
        .unwrap();
    assert!(!p.centered);
    assert_eq!((p.x, p.y), (0, 0));
    assert_eq!(layer_bytes(&same, p.layer, Channel::Color), rgba);
}

#[test]
fn a_clipboard_pastes_into_any_channel_and_a_user_channel() {
    let mut d = doc(W, H);
    let user = d
        .add_channel(ChannelInfo {
            name: "Mask A".into(),
            kind: ChannelKind::Scalar,
            color_space: ColorSpace::Linear,
            default: Rgba8::new(0, 0, 0, 255),
        })
        .unwrap();
    let l = d.add_layer("l").unwrap();
    d.set_channel_enabled(l, user, true).unwrap();
    d.set_channel_enabled(l, Channel::Normal, true).unwrap();
    for (c, v) in [(user, 77), (Channel::Normal, 200)] {
        d.set_channel_pixel(l, c, 9, 9, Rgba8::new(v, v, 255, 255))
            .unwrap();
    }
    for c in [user, Channel::Normal] {
        let copied = d.copy_pixels(l, c, false, MAX).unwrap();
        assert_eq!(copied.channel(), c);
        let pasted = d.paste_as_layer(&copied, c, None, None).unwrap();
        assert_eq!(layer_bytes(&d, pasted.layer, c), layer_bytes(&d, l, c));
        let pasted_layer = d.layer(pasted.layer).unwrap();
        assert_eq!(pasted_layer.surface_channels(), vec![c]);
        // 別のチャンネルへも貼れる（そのチャンネルだけを持つ）
        let other = if c == user {
            Channel::Metallic
        } else {
            Channel::Height
        };
        let again = d.paste_as_layer(&copied, other, None, None).unwrap();
        assert_eq!(
            d.layer(again.layer).unwrap().surface_channels(),
            vec![other]
        );
        assert_eq!(layer_bytes(&d, again.layer, other), layer_bytes(&d, l, c));
    }
    // 消したチャンネルへは貼れない（何も足さない）
    let copied = d.copy_pixels(l, user, false, MAX).unwrap();
    d.remove_channel(user).unwrap();
    let layers = d.layers().len();
    assert_eq!(
        d.paste_as_layer(&copied, user, None, None),
        Err(CoreError::ChannelNotFound)
    );
    assert_eq!(d.layers().len(), layers);
}

#[test]
fn a_disabled_channel_can_be_copied_but_not_cut() {
    let mut d = doc(W, H);
    let l = d.add_layer("l").unwrap();
    d.set_channel_pixel(l, Channel::Roughness, 3, 3, Rgba8::new(40, 40, 40, 255))
        .unwrap();
    d.set_channel_pixel(l, Channel::Roughness, 4, 4, Rgba8::new(90, 90, 90, 255))
        .unwrap();
    d.set_channel_enabled(l, Channel::Roughness, false).unwrap();
    d.clear_history().unwrap();
    let before = state(&d);
    let copied = d.copy_pixels(l, Channel::Roughness, false, MAX).unwrap();
    let r = copied.rect();
    assert_eq!(
        (r.x, r.y, r.width, r.height),
        (3, 3, 2, 2),
        "無効でも持っている画素は写せる"
    );
    assert_eq!(copied.pixel(1, 1).unwrap(), Rgba8::new(90, 90, 90, 255));
    assert_eq!(
        d.cut_pixels(l, Channel::Roughness, false, MAX),
        Err(CoreError::Unsupported("無効のチャンネルは切り取れない"))
    );
    assert_eq!(state(&d), before);
}

// ───────── ロック ─────────

#[test]
fn cut_is_refused_by_image_transparency_and_all_locks_and_nothing_is_copied() {
    for (locks, expected) in [
        (LayerLocks::PIXELS, LayerLocks::PIXELS),
        (LayerLocks::TRANSPARENCY, LayerLocks::TRANSPARENCY),
        (LayerLocks::ALL, LayerLocks::ALL),
    ] {
        for in_group in [false, true] {
            let mut d = doc(W, H);
            let l = d.add_layer("l").unwrap();
            d.set_pixel(l, 6, 6, Rgba8::new(1, 2, 3, 255)).unwrap();
            let holder = if in_group {
                d.group_layers(&[l], "g").unwrap()
            } else {
                l
            };
            d.set_layer_locks(holder, locks).unwrap();
            d.clear_history().unwrap();
            let before = state(&d);
            assert_eq!(
                d.cut_pixels(l, Channel::Color, false, MAX),
                Err(CoreError::LayerLocked {
                    layer: l,
                    holder,
                    lock: expected
                }),
                "{locks:?} in_group={in_group}"
            );
            assert_eq!(state(&d), before);
            // コピーはどのロックでもできる（文書を変えない）
            assert!(d.copy_pixels(l, Channel::Color, false, MAX).is_ok());
        }
    }
    // 位置のロックはカットを妨げない
    let mut d = doc(W, H);
    let l = d.add_layer("l").unwrap();
    d.set_pixel(l, 6, 6, Rgba8::new(1, 2, 3, 255)).unwrap();
    d.set_layer_locks(l, LayerLocks::POSITION).unwrap();
    assert!(d.cut_pixels(l, Channel::Color, false, MAX).is_ok());
    assert_eq!(px(&d, l, Channel::Color, 6, 6), Rgba8::TRANSPARENT);
}

#[test]
fn a_mask_is_cut_under_the_image_lock_but_not_under_lock_all() {
    let mut d = doc(W, H);
    let l = d.add_layer("l").unwrap();
    d.add_layer_mask(l).unwrap();
    d.set_mask_pixel(l, 5, 5, 200).unwrap();
    d.set_layer_locks(l, LayerLocks::PIXELS | LayerLocks::TRANSPARENCY)
        .unwrap();
    assert!(d.cut_pixels(l, Channel::Color, true, MAX).is_ok());
    assert_eq!(
        d.layer(l)
            .unwrap()
            .mask()
            .unwrap()
            .surface()
            .pixel(5, 5)
            .unwrap()
            .a,
        0
    );
    d.undo().unwrap();
    d.set_layer_locks(l, LayerLocks::ALL).unwrap();
    d.clear_history().unwrap();
    let before = state(&d);
    assert_eq!(
        d.cut_pixels(l, Channel::Color, true, MAX),
        Err(CoreError::LayerLocked {
            layer: l,
            holder: l,
            lock: LayerLocks::ALL
        })
    );
    assert_eq!(state(&d), before);
    // マスクの無いレイヤーは、ロックがあっても先にロックで断られる（C# の CutPixels の順）
    let bare = d.add_layer("bare").unwrap();
    d.set_layer_locks(bare, LayerLocks::ALL).unwrap();
    assert!(matches!(
        d.cut_pixels(bare, Channel::Color, true, MAX),
        Err(CoreError::LayerLocked { .. })
    ));
    d.set_layer_locks(bare, LayerLocks::NONE).unwrap();
    assert_eq!(
        d.cut_pixels(bare, Channel::Color, true, MAX),
        Err(CoreError::Unsupported("レイヤーにマスクが無い"))
    );
}

#[test]
fn a_cut_over_the_stroke_budget_changes_nothing_and_the_copy_is_not_returned() {
    let mut d = doc(64, 64);
    let l = d.add_layer("l").unwrap();
    for y in 0..64 {
        for x in 0..64 {
            d.set_pixel(l, x, y, Rgba8::new((x * 3) as u8, (y * 3) as u8, 9, 255))
                .unwrap();
        }
    }
    d.clear_history().unwrap();
    let before = state(&d);
    d.set_stroke_budget_bytes(1024).unwrap();
    assert!(matches!(
        d.cut_pixels(l, Channel::Color, false, MAX),
        Err(CoreError::StrokeBudgetExceeded)
    ));
    assert_eq!(state(&d), before);
}

// ───────── 置き換え ─────────

#[test]
fn replace_pixels_copies_the_image_exactly_inside_the_selection() {
    let mut d = doc(64, 64);
    let layer = d.add_layer("Layer").unwrap();
    d.clear_history().unwrap();
    // 右半分は透明だが RGB がある
    let img = image(64, 64, |x, y| {
        if x < 32 {
            Rgba8::new(x as u8, y as u8, 7, 255)
        } else {
            Rgba8::new(9, 8, 7, 0)
        }
    });
    assert!(d.replace_pixels(layer, Channel::Color, &img, true).unwrap());
    assert_eq!(
        px(&d, layer, Channel::Color, 25, 12),
        Rgba8::new(25, 12, 7, 255)
    );
    assert_eq!(
        px(&d, layer, Channel::Color, 50, 12),
        Rgba8::new(9, 8, 7, 0),
        "透明画素も RGB を保つ"
    );
    assert!(
        !d.replace_pixels(layer, Channel::Color, &img, true).unwrap(),
        "2 回目は何も変わらず、段も積まない"
    );
    assert_eq!(d.undo_count(), 1);
    d.undo().unwrap();
    assert_eq!(px(&d, layer, Channel::Color, 25, 12).a, 0);

    // 選択範囲: 中は置き換え、外はそのまま。選択を無視することもできる
    d.set_selection(Some(SelectionMask::rectangle(&d, 0, 0, 16, 16)))
        .unwrap();
    let red = image(64, 64, |_, _| Rgba8::new(255, 0, 0, 255));
    d.replace_pixels(layer, Channel::Color, &red, true).unwrap();
    assert_eq!(
        px(&d, layer, Channel::Color, 10, 10),
        Rgba8::new(255, 0, 0, 255)
    );
    assert_eq!(px(&d, layer, Channel::Color, 30, 30).a, 0);
    d.replace_pixels(layer, Channel::Color, &red, false)
        .unwrap();
    assert_eq!(
        px(&d, layer, Channel::Color, 30, 30),
        Rgba8::new(255, 0, 0, 255)
    );
}

#[test]
fn a_partial_selection_interpolates_in_premultiplied_space() {
    let mut d = doc(64, 64);
    let layer = d.add_layer("Layer").unwrap();
    d.fill(
        layer,
        Channel::Color,
        Rgba8::new(0, 0, 255, 255),
        1.0,
        None,
        false,
    )
    .unwrap();
    // どこも 128/255 だけ選ばれた選択範囲
    let ts = d.tile_size();
    let tiles: Vec<_> = d
        .canvas_tiles()
        .map(|c| (c, vec![128u8; (ts * ts) as usize]))
        .collect();
    let half = SelectionMask::from_amount_tiles(64, 64, ts, tiles).unwrap();
    d.set_selection(Some(half)).unwrap();
    let transparent_red = image(64, 64, |_, _| Rgba8::new(255, 0, 0, 0));
    d.replace_pixels(layer, Channel::Color, &transparent_red, true)
        .unwrap();
    let p = px(&d, layer, Channel::Color, 5, 5);
    assert!((126..=129).contains(&p.a), "透明へ半分: {p:?}");
    assert_eq!(p.b, 255, "透明な元は色を足さない: 残った覆いは青のまま");
    assert_eq!(p.r, 0);
}

#[test]
fn replace_pixels_refuses_wrong_sizes_and_layers_without_pixels() {
    let mut d = doc(64, 64);
    let layer = d.add_layer("Layer").unwrap();
    let group = d.add_group("Group", None).unwrap();
    let fill = d.add_fill_layer("Fill", &[], None).unwrap();
    let image = vec![0u8; 64 * 64 * 4];
    assert_eq!(
        d.replace_pixels(layer, Channel::Color, &[0; 16], true),
        Err(CoreError::InvalidArgument("画像の大きさがキャンバスと違う"))
    );
    assert!(matches!(
        d.replace_pixels(group, Channel::Color, &image, true),
        Err(CoreError::Unsupported(_))
    ));
    assert!(matches!(
        d.replace_pixels(fill, Channel::Color, &image, true),
        Err(CoreError::Unsupported(_))
    ));
    assert_eq!(
        d.replace_pixels(layer, Channel::Roughness, &image, true),
        Err(CoreError::Unsupported("無効のチャンネルは置き換えられない"))
    );
    assert_eq!(
        d.replace_pixels(LayerId(5), Channel::Color, &image, true),
        Err(CoreError::LayerNotFound)
    );
    // 画素の予算（作った面を残さない）
    let mut small = doc(64, 64);
    small.set_source_budget_bytes(1024).unwrap();
    let l2 = small.add_layer("L").unwrap();
    let (revision, undo) = (small.revision(), small.undo_count());
    let big = image_of(64, 64);
    assert_eq!(
        small.replace_pixels(l2, Channel::Color, &big, true),
        Err(CoreError::SourceBudgetExceeded)
    );
    assert_eq!(
        px(&small, l2, Channel::Color, 0, 0).a,
        0,
        "予算を超えたら何も変えない"
    );
    assert_eq!(small.revision(), revision);
    assert_eq!(small.undo_count(), undo, "段を積まない");
}
fn image_of(w: u32, h: u32) -> Vec<u8> {
    image(w, h, |x, y| Rgba8::new(x as u8, y as u8, 3, 255))
}

#[test]
fn replace_pixels_obeys_the_locks() {
    let mut d = doc(32, 32);
    let l = d.add_layer("l").unwrap();
    for y in 0..32 {
        for x in 0..32 {
            let a = match (x + y) % 3 {
                0 => 0,
                1 => 100,
                _ => 255,
            };
            d.set_pixel(l, x, y, Rgba8::new(10, 20, 30, a)).unwrap();
        }
    }
    d.clear_history().unwrap();
    let img = image_of(32, 32);
    for (locks, holder_group) in [
        (LayerLocks::PIXELS, false),
        (LayerLocks::ALL, false),
        (LayerLocks::PIXELS, true),
        (LayerLocks::ALL, true),
    ] {
        let mut d2 = doc(32, 32);
        let l2 = d2.add_layer("l").unwrap();
        d2.set_pixel(l2, 1, 1, Rgba8::new(1, 1, 1, 255)).unwrap();
        let holder = if holder_group {
            d2.group_layers(&[l2], "g").unwrap()
        } else {
            l2
        };
        d2.set_layer_locks(holder, locks).unwrap();
        d2.clear_history().unwrap();
        let b = state(&d2);
        assert_eq!(
            d2.replace_pixels(l2, Channel::Color, &image_of(32, 32), true),
            Err(CoreError::LayerLocked {
                layer: l2,
                holder,
                lock: locks
            })
        );
        assert_eq!(state(&d2), b);
    }
    // 透明部分のロック: 各画素のアルファを保ち、透明な画素はそのまま、色だけが置き換わる
    d.set_layer_locks(l, LayerLocks::TRANSPARENCY).unwrap();
    d.clear_history().unwrap();
    let before = shape(&d);
    assert!(d.replace_pixels(l, Channel::Color, &img, true).unwrap());
    for y in 0..32 {
        for x in 0..32 {
            let old = Rgba8::new(10, 20, 30, [0, 100, 255][((x + y) % 3) as usize]);
            let now = px(&d, l, Channel::Color, x, y);
            assert_eq!(now.a, old.a, "アルファを保つ ({x}, {y})");
            if old.a == 0 {
                assert_eq!(now, old, "透明な画素はそのまま");
            } else {
                assert_eq!((now.r, now.g, now.b), (x as u8, y as u8, 3));
            }
        }
    }
    d.undo().unwrap();
    assert_eq!(shape(&d), before);
    // 位置のロックは妨げない
    d.set_layer_locks(l, LayerLocks::POSITION).unwrap();
    assert!(d.replace_pixels(l, Channel::Color, &img, true).unwrap());
}

// ───────── まとめ（batch） ─────────

#[test]
fn a_batch_is_one_undo_step_that_redoes_as_a_whole() {
    let mut d = doc(64, 64);
    let first = d.add_layer("First").unwrap();
    d.clear_history().unwrap();
    let added = d
        .batch(|d| {
            let layer = d.add_layer("Added")?;
            d.set_channel_enabled(layer, Channel::Color, true)?;
            d.fill(
                layer,
                Channel::Color,
                Rgba8::new(255, 0, 0, 255),
                1.0,
                None,
                false,
            )?;
            d.set_layer_opacity(layer, 0.5, false)?;
            Ok(layer)
        })
        .unwrap();
    assert_eq!(d.layers().len(), 2);
    assert_eq!(d.undo_count(), 1);
    assert!(d.undo().unwrap());
    assert_eq!(
        d.layers().iter().map(|l| l.id()).collect::<Vec<_>>(),
        vec![first],
        "Undo 1 回でまとめ全部が戻る"
    );
    assert!(!d.can_undo());
    assert!(d.redo().unwrap());
    assert_eq!(d.layer(added).unwrap().opacity(), 0.5);
    assert_eq!(
        px(&d, added, Channel::Color, 10, 10),
        Rgba8::new(255, 0, 0, 255)
    );
    assert!(!d.is_batching());
}

#[test]
fn a_batch_that_adds_no_step_adds_none_and_keeps_redo() {
    let mut d = doc(32, 32);
    let l = d.add_layer("l").unwrap();
    d.set_layer_opacity(l, 0.3, false).unwrap();
    d.undo().unwrap();
    let (undo, redo, bytes) = (d.undo_count(), d.redo_count(), d.history_bytes());
    d.batch(|_| Ok(())).unwrap();
    assert_eq!(
        (d.undo_count(), d.redo_count(), d.history_bytes()),
        (undo, redo, bytes)
    );
    assert!(d.redo().unwrap());
    assert_eq!(d.layer(l).unwrap().opacity(), 0.3);
    // 1 つだけ積んだまとめは、普通の段（まとめの印を残さない）
    d.batch(|d| d.set_layer_opacity(l, 0.8, false)).unwrap();
    assert!(d.undo().unwrap());
    assert_eq!(d.layer(l).unwrap().opacity(), 0.3);
}

#[test]
fn a_failing_batch_leaves_the_document_and_history_as_before() {
    let mut d = doc(64, 64);
    let layer = d.add_layer("Layer").unwrap();
    d.fill(
        layer,
        Channel::Color,
        Rgba8::new(0, 0, 255, 255),
        1.0,
        None,
        false,
    )
    .unwrap();
    d.set_layer_opacity(layer, 0.7, false).unwrap();
    d.undo().unwrap(); // 1 段のやり直しを残す
    let (revision, history) = (d.revision(), d.history_bytes());
    let before = d.composite(d.bounds()).unwrap();
    let before_content = content(&d);
    let result: Result<(), CoreError> = d.batch(|d| {
        d.fill(
            layer,
            Channel::Color,
            Rgba8::new(0, 255, 0, 255),
            1.0,
            None,
            false,
        )?;
        d.add_layer("Doomed")?;
        Err(CoreError::InvalidArgument("boom"))
    });
    assert_eq!(result, Err(CoreError::InvalidArgument("boom")));
    assert_eq!(d.layers().len(), 1);
    assert_eq!(d.composite(d.bounds()).unwrap(), before, "画素は元に戻る");
    assert_eq!(d.history_bytes(), history);
    assert!(
        d.revision() > revision,
        "変更番号は進み、表示は描き直される"
    );
    assert!(d.can_redo(), "まとめが落としたはずのやり直しが残る");
    assert_eq!(content(&d), before_content);
    assert!(d.redo().unwrap());
    assert_eq!(d.layer(layer).unwrap().opacity(), 0.7);
    assert!(!d.is_batching());
}

#[test]
fn a_panicking_batch_is_rolled_back_and_does_not_stay_batching() {
    let mut d = doc(32, 32);
    let l = d.add_layer("l").unwrap();
    d.clear_history().unwrap();
    let before = content(&d);
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        let _ = d.batch(|d| -> Result<(), CoreError> {
            d.set_layer_opacity(l, 0.1, false)?;
            panic!("まとめの中の panic");
        });
    }));
    assert!(outcome.is_err());
    assert!(!d.is_batching());
    assert_eq!(content(&d), before);
    assert!(
        d.set_layer_opacity(l, 0.2, false).is_ok(),
        "文書はまた使える"
    );
    assert!(d.undo().unwrap());
}

#[test]
fn undoing_an_enabled_channel_leaves_the_document_as_before() {
    let mut d = doc(64, 64);
    let layer = d.add_layer("Layer").unwrap();
    d.clear_history().unwrap();
    let before = shape(&d);
    d.set_channel_enabled(layer, Channel::Roughness, true)
        .unwrap();
    assert!(d.undo().unwrap());
    assert_eq!(shape(&d), before, "空の Roughness の面が残らない");
    assert!(d.redo().unwrap());
    assert!(d
        .layer(layer)
        .unwrap()
        .is_channel_enabled(Channel::Roughness));
    // まとめが失敗して取り消されたときも同じ
    d.undo().unwrap();
    let after_undo = content(&d);
    let result: Result<(), CoreError> = d.batch(|d| {
        d.set_channel_enabled(layer, Channel::Metallic, true)?;
        Err(CoreError::InvalidArgument("x"))
    });
    assert!(result.is_err());
    assert_eq!(content(&d), after_undo);
    assert_eq!(shape(&d), before);
}

#[test]
fn batches_refuse_nesting_strokes_undo_and_direct_writes() {
    let mut d = doc(64, 64);
    let layer = d.add_layer("Layer").unwrap();
    d.set_layer_opacity(layer, 0.4, false).unwrap();
    d.undo().unwrap();
    let (history, revision, undo) = (d.history_bytes(), d.revision(), d.undo_count());
    let color = Rgba8::new(1, 2, 3, 255);
    let paint = [ChannelPaint::new(Channel::Color, color)];
    // まとめの外では、次の 3 つは別の理由で断る（まとめの中では、確かめより先に BatchActive）
    assert_eq!(
        d.begin_triangle_fill(layer, Channel::Roughness, color, 1.0, false)
            .map(drop),
        Err(CoreError::Unsupported("無効のチャンネルには塗れない"))
    );
    assert_eq!(
        d.begin_triangle_fill(layer, Channel::from_index(40).unwrap(), color, 1.0, false)
            .map(drop),
        Err(CoreError::ChannelNotFound)
    );
    assert_eq!(
        d.begin_triangle_fill(LayerId(99), Channel::Color, color, 1.0, false)
            .map(drop),
        Err(CoreError::LayerNotFound)
    );
    type Case = Box<dyn Fn(&mut Document) -> Result<(), CoreError>>;
    let cases: Vec<Case> = vec![
        Box::new(|d| d.batch(|_| Ok(()))),
        Box::new(move |d| d.begin_stroke(layer, &BrushSettings::default()).map(drop)),
        Box::new(|d| d.undo().map(drop)),
        Box::new(|d| d.redo().map(drop)),
        Box::new(|d| d.clear_history()),
        Box::new(move |d| d.set_pixel(layer, 1, 1, Rgba8::new(1, 2, 3, 255)).map(drop)),
        Box::new(move |d| d.set_mask_pixel(layer, 1, 1, 3).map(drop)),
        Box::new(move |d| d.set_locks_for_load(layer, LayerLocks::NONE)),
        Box::new(|d| d.cancel_coalescing().map(drop)),
        // ストロークを始める入口（マスク・マテリアル・三角形の塗り）
        Box::new(move |d| {
            d.begin_mask_stroke(layer, &BrushSettings::default())
                .map(drop)
        }),
        Box::new(move |d| {
            d.begin_brush_mask_stroke(layer, &Brush::from(BrushSettings::default()))
                .map(drop)
        }),
        Box::new(move |d| {
            d.begin_material_stroke(layer, &paint, &BrushSettings::default())
                .map(drop)
        }),
        Box::new(move |d| {
            d.begin_material_brush_stroke(layer, &paint, &Brush::from(BrushSettings::default()))
                .map(drop)
        }),
        Box::new(move |d| {
            d.begin_triangle_fill(layer, Channel::Color, color, 1.0, false)
                .map(drop)
        }),
        Box::new(move |d| {
            d.begin_material_triangle_fill(layer, &paint, 1.0, false)
                .map(drop)
        }),
        Box::new(move |d| d.begin_mask_triangle_fill(layer, 1.0, false).map(drop)),
        // 三角形の塗りは、無効なチャンネル・無いチャンネル・無いレイヤーでも、まとめの中の断りが先
        Box::new(move |d| {
            d.begin_triangle_fill(layer, Channel::Roughness, color, 1.0, false)
                .map(drop)
        }),
        Box::new(move |d| {
            d.begin_triangle_fill(layer, Channel::from_index(40).unwrap(), color, 1.0, false)
                .map(drop)
        }),
        Box::new(move |d| {
            d.begin_triangle_fill(LayerId(99), Channel::Color, color, 1.0, false)
                .map(drop)
        }),
        // 読み込み用の直接の書き込み
        Box::new(move |d| {
            d.import_tile(layer, Channel::Color, TileCoord::new(0, 0), &[])
                .map(drop)
        }),
        Box::new(move |d| {
            d.import_mask_tile(layer, TileCoord::new(0, 0), &[])
                .map(drop)
        }),
        Box::new(|d| {
            let info = ChannelInfo {
                name: "Loaded".into(),
                kind: ChannelKind::Scalar,
                color_space: ColorSpace::Linear,
                default: Rgba8::new(0, 0, 0, 255),
            };
            d.insert_channel_for_load(Channel::from_index(20).unwrap(), info)
        }),
        Box::new(|d| d.set_structure_for_load(&[None])),
    ];
    for (i, case) in cases.iter().enumerate() {
        let result = d.batch(|d| case(d));
        assert_eq!(result, Err(CoreError::BatchActive), "case {i}");
        assert!(!d.is_batching(), "case {i}");
        assert_eq!(
            d.history_bytes(),
            history,
            "case {i}: 拒んだまとめは履歴を残さない"
        );
        assert_eq!(d.revision(), revision, "case {i}");
        assert!(d.can_redo(), "case {i}");
        assert_eq!(d.undo_count(), undo, "case {i}");
    }
    // 外から見ると、まとめでないときは今までどおり
    assert!(d.set_pixel(layer, 1, 1, Rgba8::new(1, 2, 3, 255)).is_ok());
}

#[test]
fn history_is_trimmed_only_after_the_batch_so_a_failure_can_still_undo_everything() {
    let mut d = doc(64, 64);
    let l = d.add_layer("l").unwrap();
    d.set_undo_budget_bytes(1).unwrap();
    d.clear_history().unwrap();
    let before = shape(&d);
    // 予算が 1 バイトでも、まとめの途中では古い段を落とさない（落とすと失敗したときに戻せない）
    let result: Result<(), CoreError> = d.batch(|d| {
        for i in 0..5 {
            d.fill(
                l,
                Channel::Color,
                Rgba8::new(i * 40, 0, 0, 255),
                1.0,
                None,
                false,
            )?;
        }
        assert_eq!(d.undo_count(), 5);
        Err(CoreError::InvalidArgument("x"))
    });
    assert!(result.is_err());
    assert_eq!(
        content(&d),
        State {
            revision: 0,
            ..state(&d)
        }
    );
    assert_eq!(shape(&d), before);
    assert_eq!(d.history_bytes(), 0);
    // 成功すれば 1 段にまとまり、そのあとで整理される（最新の段は予算を超えても残る）
    d.set_minimum_undo_steps(1).unwrap();
    d.batch(|d| {
        for i in 0..5 {
            d.fill(
                l,
                Channel::Color,
                Rgba8::new(i * 40, 0, 0, 255),
                1.0,
                None,
                false,
            )?;
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(d.undo_count(), 1);
    assert!(d.undo().unwrap());
    assert_eq!(shape(&d), before);
}

/// `with_persistent_ids` は文書を値で受けるので、まとめの中（`&mut Document`）からは、文書を取り出したときだけ届く（そのときも
/// 断る）。
#[test]
fn a_document_taken_out_of_a_batch_cannot_get_persistent_ids() {
    let mut d = doc(16, 16);
    let layer = d.add_layer("l").unwrap();
    d.clear_history().unwrap();
    let outcome = d.batch(|inside| {
        let taken = std::mem::replace(inside, doc(16, 16));
        assert!(taken.is_batching());
        taken.with_persistent_ids(7, &[layer]).map(drop)
    });
    assert_eq!(outcome, Err(CoreError::BatchActive));
    assert!(!d.is_batching());
}

/// レイヤーを足す段と、画素を持つレイヤーを貼る段を 1 回にまとめた batch を Undo して、画素の予算を今の画素のすぐ上まで下げると、Redo は
/// 2 段目で予算に断られる: 済んだ 1 段目（レイヤーを足す）は戻り、Redo の段は残り、文書も履歴も変わらない。
#[test]
fn a_redo_refused_at_the_second_step_of_a_batch_puts_back_the_first() {
    let mut seed = 21;
    let mut d = doc(W, H);
    let base = d.add_layer("base").unwrap();
    scatter(&mut d, base, Channel::Color, &mut seed, 300);
    d.clear_history().unwrap();
    let clip = d.copy_pixels(base, Channel::Color, false, MAX).unwrap();
    let (first, pasted) = d
        .batch(|d| {
            let first = d.add_layer("empty")?;
            let pasted = d.paste_as_layer(&clip, Channel::Color, Some("pasted"), None)?;
            Ok((first, pasted.layer))
        })
        .unwrap();
    assert_eq!(d.undo_count(), 1, "2 つの操作が 1 段にまとまる");
    let first_bytes = d.layer(first).unwrap().allocated_bytes();
    let pasted_bytes = d.layer(pasted).unwrap().allocated_bytes();
    assert!(pasted_bytes > 0);
    assert!(d.undo().unwrap());
    assert!(d.layer(first).is_none() && d.layer(pasted).is_none());
    let before = state(&d);
    let budget = d.source_budget_bytes();
    // 1 段目の空のレイヤーは入り、2 段目の貼ったレイヤーは 1 バイト足りない
    d.set_source_budget_bytes(d.allocated_bytes() + first_bytes + pasted_bytes - 1)
        .unwrap();
    assert_eq!(d.redo(), Err(CoreError::SourceBudgetExceeded));
    assert_eq!(
        state(&d),
        before,
        "レイヤーも画素も選択も、Undo の数・Redo の数・履歴のバイト数・変更番号も、断る前のまま"
    );
    assert!(d.can_redo() && d.layer(first).is_none());
    // 予算を戻せば、同じ段がそのままやり直せる
    d.set_source_budget_bytes(budget).unwrap();
    assert!(d.redo().unwrap());
    assert!(d.layer(first).is_some());
    assert_eq!(
        layer_bytes(&d, pasted, Channel::Color),
        layer_bytes(&d, base, Channel::Color)
    );
    assert_eq!(d.undo_count(), 1);
}

/// 戻す向き: 画素を消す段とレイヤーを足す段をまとめた batch の Undo は、新しい方（レイヤーを足す段）から戻す。2 段目（消した画素を戻す）が
/// 予算に断られたら、済んだレイヤーを足す段をもう一度当て直して、文書は Undo の前のまま。
#[test]
fn an_undo_refused_at_the_second_step_of_a_batch_applies_the_first_again() {
    let mut seed = 22;
    let mut d = doc(W, H);
    let base = d.add_layer("base").unwrap();
    scatter(&mut d, base, Channel::Color, &mut seed, 300);
    d.clear_history().unwrap();
    let late = d
        .batch(|d| {
            d.cut_pixels(base, Channel::Color, false, MAX)?;
            d.add_layer("late")
        })
        .unwrap();
    assert_eq!(d.undo_count(), 1);
    assert_eq!(
        layer_bytes(&d, base, Channel::Color),
        vec![0; (W * H * 4) as usize]
    );
    let before = state(&d);
    // 画素を戻す分の予算が無い（レイヤーを取り除く段の分は空く）
    d.set_source_budget_bytes(d.allocated_bytes()).unwrap();
    assert_eq!(d.undo(), Err(CoreError::SourceBudgetExceeded));
    assert_eq!(
        state(&d),
        before,
        "取り除いたレイヤーは戻り、消した画素は消えたまま、履歴も変わらない"
    );
    assert!(d.layer(late).is_some() && d.can_undo() && !d.can_redo());
    // 予算を戻せば Undo できて、画素もレイヤーも元へ戻る
    d.set_source_budget_bytes(64 << 20).unwrap();
    assert!(d.undo().unwrap());
    assert!(d.layer(late).is_none());
    assert_ne!(
        layer_bytes(&d, base, Channel::Color),
        vec![0; (W * H * 4) as usize]
    );
}

#[test]
fn a_batch_of_clipboard_operations_undoes_as_one() {
    let mut d = doc(W, H);
    let l = d.add_layer("l").unwrap();
    for y in 0..12 {
        for x in 0..12 {
            d.set_pixel(l, x, y, Rgba8::new(x as u8 * 9, y as u8 * 9, 5, 255))
                .unwrap();
        }
    }
    d.set_selection(Some(SelectionMask::rectangle(&d, 0, 0, 6, 6)))
        .unwrap();
    d.clear_history().unwrap();
    let before = shape(&d);
    let (clip, pasted) = d
        .batch(|d| {
            let clip = d.cut_pixels(l, Channel::Color, false, u64::MAX)?;
            let pasted = d.paste_as_layer(&clip, Channel::Color, Some("moved"), Some(l))?;
            d.set_layer_opacity(pasted.layer, 0.5, false)?;
            Ok((clip, pasted))
        })
        .unwrap();
    assert_eq!(clip.rect().width, 6);
    assert_eq!(
        d.undo_count(),
        1,
        "カット・貼り付け（レイヤーと選択）・不透明度が 1 回の Undo"
    );
    assert_eq!(px(&d, l, Channel::Color, 2, 2), Rgba8::TRANSPARENT);
    assert_eq!(
        px(&d, pasted.layer, Channel::Color, 2, 2),
        Rgba8::new(18, 18, 5, 255)
    );
    assert!(d.undo().unwrap());
    assert_eq!(shape(&d), before);
    assert!(d.redo().unwrap());
    assert_eq!(d.layer(pasted.layer).unwrap().opacity(), 0.5);
    assert!(d.selection().is_none());
    // まとめの中で貼って失敗したら、貼ったレイヤーも選択の解除も戻る
    d.undo().unwrap();
    let after_undo = content(&d);
    let result: Result<(), CoreError> = d.batch(|d| {
        let clip = d.copy_pixels(l, Channel::Color, false, u64::MAX)?;
        d.paste_as_layer(&clip, Channel::Color, None, None)?;
        d.replace_pixels(l, Channel::Color, &[1, 2, 3], true)?; // 大きさが違う
        Ok(())
    });
    assert!(result.is_err());
    assert_eq!(content(&d), after_undo);
    assert_eq!(shape(&d), before);
}

#[test]
fn replace_pixels_in_a_batch_with_a_second_replace_returns_to_the_start_in_one_undo() {
    let mut d = doc(32, 32);
    let l = d.add_layer("l").unwrap();
    d.clear_history().unwrap();
    let start = shape(&d);
    let a = image_of(32, 32);
    let b = image(32, 32, |x, y| Rgba8::new(y as u8, x as u8, 9, 200));
    d.batch(|d| {
        d.replace_pixels(l, Channel::Color, &a, true)?;
        d.replace_pixels(l, Channel::Color, &b, true)?;
        Ok(())
    })
    .unwrap();
    assert_eq!(d.undo_count(), 1);
    assert_eq!(layer_bytes(&d, l, Channel::Color), b);
    assert!(d.undo().unwrap());
    assert_eq!(shape(&d), start);
    assert!(d.redo().unwrap());
    assert_eq!(layer_bytes(&d, l, Channel::Color), b);
}

#[test]
fn undo_tile_changes_are_reported_to_the_journal_for_paste_cut_and_replace() {
    // 表示が描き直せるよう、貼る・切る・置き換える・戻すで、合成が変わるタイルが変化の記録に出る
    let mut d = doc(W, H);
    let l = d.add_layer("l").unwrap();
    d.set_pixel(l, 20, 20, Rgba8::new(1, 1, 1, 255)).unwrap();
    let copied = d.copy_pixels(l, Channel::Color, false, MAX).unwrap();
    let since = d.change_serial();
    let pasted = d
        .paste_as_layer(&copied, Channel::Color, None, None)
        .unwrap();
    let tile = TileCoord::new(20 / TILE, 20 / TILE);
    assert!(d
        .changed_tiles(Channel::Color, since)
        .unwrap()
        .contains(&tile));
    let since = d.change_serial();
    d.undo().unwrap();
    assert!(d
        .changed_tiles(Channel::Color, since)
        .unwrap()
        .contains(&tile));
    let since = d.change_serial();
    d.cut_pixels(l, Channel::Color, false, MAX).unwrap();
    assert!(d
        .changed_tiles(Channel::Color, since)
        .unwrap()
        .contains(&tile));
    let since = d.change_serial();
    d.replace_pixels(l, Channel::Color, &image_of(W, H), true)
        .unwrap();
    assert!(d.changed_tiles(Channel::Color, since).unwrap().len() >= 6);
    let _ = pasted;
}
