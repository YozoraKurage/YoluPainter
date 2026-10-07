//! 層の画素のコピー・カット・結合してコピー・ペースト（編集メニューとキー）と、OS のクリップボードの画像のやり取り。OS 側は
//! メモリ上の差し替え（`MemoryClipboard`）で、本物の OS のクリップボードには触れない。
//! `headless_` で始まる試験は画面を描かず、Wine でも回る。
use crate::common;

use common::*;
use egui::{Event, Key, Modifiers};
use egui_kittest::Harness;
use yolu_app::clipboard::os::{ClipImage, MemoryClipboard, OsClipboardError};
use yolu_app::clipboard::{menu_entries, ClipAction};
use yolu_app::engine::{Channel, ClipboardSource, LayerId, Rgba8, SelectionMask};
use yolu_app::lang::Lang;
use yolu_app::state::{Action, AppState};
use yolu_app::ui::menu::Entry;
use yolu_app::YoluApp;
use yolu_core::LayerLocks;

type H = Harness<'static, YoluApp>;

// ───────── 道具 ─────────

/// 64 × 64 の文書。最初の層に (8..40, 10..30) の不透明な画素と、透明だが RGB のある画素 (9, 11) がある。履歴は空。OS は差し替え。
fn painted() -> (AppState, MemoryClipboard, LayerId) {
    let mut s = AppState::new(64, 64);
    let os = MemoryClipboard::new();
    s.clip.set_os(Box::new(os.clone()));
    let id = s.selected_layer.unwrap();
    for y in 10..30 {
        for x in 8..40 {
            s.doc
                .set_pixel(id, x, y, Rgba8::new(x as u8 * 5, y as u8 * 7, 100, 255))
                .unwrap();
        }
    }
    s.doc
        .set_pixel(id, 9, 11, Rgba8::new(10, 20, 30, 0))
        .unwrap();
    s.doc.clear_history().unwrap();
    (s, os, id)
}

fn run(s: &mut AppState, action: ClipAction) {
    s.apply(Action::Clip(action));
}

fn select(s: &mut AppState, x0: i64, y0: i64, x1: i64, y1: i64) {
    let mask = SelectionMask::rectangle(&s.doc, x0, y0, x1, y1);
    s.doc.set_selection(Some(mask)).unwrap();
    s.doc.clear_history().unwrap();
}

fn px(s: &AppState, id: LayerId, x: u32, y: u32) -> Rgba8 {
    s.doc
        .layer(id)
        .unwrap()
        .pixel(Channel::Color, x, y)
        .unwrap()
}

/// 外のアプリがコピーした画像（straight RGBA8、上の行から）。
fn external(width: u32, height: u32) -> ClipImage {
    let mut rgba = Vec::new();
    for row in 0..height {
        for col in 0..width {
            rgba.extend([(col * 20 + 1) as u8, (row * 30 + 2) as u8, 77, 255]);
        }
    }
    ClipImage::new(width, height, rgba).unwrap()
}

fn st(h: &H) -> &AppState {
    &h.state().state
}

fn item(entries: &[Entry<Action>], action: ClipAction) -> (&str, Option<&str>, bool) {
    entries
        .iter()
        .find_map(|e| match e {
            Entry::Item {
                label,
                shortcut,
                enabled,
                action: Action::Clip(a),
                ..
            } if *a == action => Some((label.as_str(), shortcut.as_deref(), *enabled)),
            _ => None,
        })
        .unwrap()
}

// ───────── コピー・カット・ペースト ─────────

#[test]
fn headless_copy_keeps_the_selection_in_the_app_clipboard_and_writes_the_image_to_the_os() {
    let (mut s, os, _) = painted();
    select(&mut s, 8, 10, 20, 20);
    run(&mut s, ClipAction::Copy);
    let clip = s.clip.pixels.as_ref().expect("アプリの中の写し");
    let r = clip.rect();
    assert_eq!((r.x, r.y, r.width, r.height), (8, 10, 12, 10));
    assert_eq!(clip.source(), ClipboardSource::Layer);
    assert!(s.message.contains("12 × 10"), "{}", s.message);
    assert_eq!(s.doc.undo_count(), 0, "コピーは文書を変えない");
    assert!(!s.modified);
    // OS へは同じ画素が、上の行から
    let image = os.get().expect("OS へ書いた");
    assert_eq!((image.width, image.height), (12, 10));
    let row = 12 * 4;
    assert_eq!(
        &image.rgba[..row],
        &clip.pixels()[9 * row..10 * row],
        "最後の行が上"
    );
    assert_eq!(&image.rgba[9 * row..], &clip.pixels()[..row]);
    assert_eq!(os.counts().0, 1);
}

#[test]
fn headless_cut_is_one_undo_and_a_paste_puts_it_back_as_a_new_layer_above() {
    let (mut s, _os, id) = painted();
    select(&mut s, 8, 10, 20, 20);
    let original = px(&s, id, 12, 15);
    run(&mut s, ClipAction::Cut);
    assert_eq!(px(&s, id, 12, 15), Rgba8::TRANSPARENT);
    assert_eq!(px(&s, id, 30, 25).a, 255, "選択の外はそのまま");
    assert_eq!(s.doc.undo_count(), 1);
    assert!(s.modified);
    assert!(s.message.contains("カット"), "{}", s.message);

    run(&mut s, ClipAction::Paste);
    assert_eq!(s.doc.layers().len(), 2);
    let pasted = s.selected_layer.unwrap();
    assert_ne!(pasted, id, "貼った層を選ぶ");
    assert_eq!(s.doc.layer_index(pasted), Some(1), "選んでいた層のすぐ上");
    assert_eq!(px(&s, pasted, 12, 15), original, "元の位置に戻る");
    assert_eq!(s.doc.layer(pasted).unwrap().name(), "レイヤー 2");
    assert!(s.doc.selection().is_none(), "貼ると選択を外す");
    assert_eq!(
        s.doc.undo_count(),
        2,
        "貼り付けは層を足すことと選択を外すことで 1 回"
    );
    assert!(s.message.contains("ペースト"), "{}", s.message);

    s.apply(Action::Undo);
    assert_eq!(s.doc.layers().len(), 1);
    assert!(s.doc.selection().is_some(), "Undo で選択も戻る");
    assert_eq!(s.selected_layer, Some(id), "消えた層の選択は一番上へ戻る");
    s.apply(Action::Undo);
    assert_eq!(px(&s, id, 12, 15), original);
}

#[test]
fn headless_paste_uses_the_app_clipboard_when_the_os_still_holds_what_was_copied() {
    let (mut s, os, id) = painted();
    run(&mut s, ClipAction::Copy);
    // ほかのアプリは透明画素の RGB を落とすことがある: それでも自分の書いた画像として、正確な写しを貼る
    let mut stripped = os.get().unwrap();
    for p in stripped.rgba.as_chunks_mut::<4>().0 {
        if p[3] == 0 {
            p[..3].fill(0);
        }
    }
    os.set(Some(stripped));
    run(&mut s, ClipAction::Paste);
    let pasted = s.selected_layer.unwrap();
    assert_ne!(pasted, id);
    assert_eq!(
        px(&s, pasted, 9, 11),
        Rgba8::new(10, 20, 30, 0),
        "透明画素の RGB を保つ"
    );
    // OS に画像が無くなった・読めないときも、アプリの中の写しを貼る
    os.set(None);
    run(&mut s, ClipAction::Paste);
    assert_eq!(s.doc.layers().len(), 3);
    os.fail_reads(Some(OsClipboardError::Unavailable));
    run(&mut s, ClipAction::Paste);
    assert_eq!(s.doc.layers().len(), 4);
}

#[test]
fn headless_paste_takes_an_external_image_centred_and_flipped() {
    let (mut s, os, id) = painted();
    run(&mut s, ClipAction::Copy);
    os.set(Some(external(8, 6)));
    run(&mut s, ClipAction::Paste);
    let pasted = s.selected_layer.unwrap();
    assert_ne!(pasted, id);
    // 64 × 64 の中央（28, 29）。OS の画像の上の行が、文書では上（y が大きい側）
    assert_eq!(
        px(&s, pasted, 28, 29 + 5),
        Rgba8::new(1, 2, 77, 255),
        "左上"
    );
    assert_eq!(
        px(&s, pasted, 28 + 7, 29),
        Rgba8::new(141, 152, 77, 255),
        "右下"
    );
    assert_eq!(px(&s, pasted, 27, 29).a, 0);
    assert_eq!(
        s.doc.layer(pasted).unwrap().surface_channels(),
        vec![Channel::Color]
    );
    assert!(s.message.contains("中央"), "{}", s.message);
    // 外の画像が自分の写しを置き換えるのではない: アプリの中の写しはそのまま
    assert_eq!(
        s.clip.pixels.as_ref().unwrap().source(),
        ClipboardSource::Layer
    );
    // 文書と同じ大きさなら (0, 0)
    os.set(Some(external(64, 64)));
    run(&mut s, ClipAction::Paste);
    let full = s.selected_layer.unwrap();
    assert_eq!(px(&s, full, 0, 63), Rgba8::new(1, 2, 77, 255));
    assert_eq!(
        px(&s, full, 63, 0),
        Rgba8::new((63u32 * 20 + 1) as u8, (63u32 * 30 + 2) as u8, 77, 255)
    );
}

#[test]
fn headless_an_external_image_larger_than_the_set_is_refused_and_changes_nothing() {
    let (mut s, os, _) = painted();
    os.set(Some(external(65, 10)));
    run(&mut s, ClipAction::Paste);
    assert_eq!(s.doc.layers().len(), 1);
    assert_eq!(s.doc.undo_count(), 0);
    assert!(
        s.message.contains("大きい") && s.message.contains("65 × 10"),
        "{}",
        s.message
    );
    os.set(Some(external(10, 65)));
    s.lang = Lang::En;
    run(&mut s, ClipAction::Paste);
    assert!(
        s.message.contains("larger than the texture set"),
        "{}",
        s.message
    );
    assert_eq!(s.doc.layers().len(), 1);
}

#[test]
fn headless_an_empty_or_unreadable_clipboard_says_so() {
    let (mut s, os, _) = painted();
    run(&mut s, ClipAction::Paste);
    assert!(s.message.contains("空"), "{}", s.message);
    os.fail_reads(Some(OsClipboardError::Busy));
    run(&mut s, ClipAction::Paste);
    assert!(s.message.contains("使用中"), "{}", s.message);
    s.lang = Lang::En;
    run(&mut s, ClipAction::Paste);
    assert_eq!(s.message, "OS clipboard is busy");
    os.fail_reads(None);
    run(&mut s, ClipAction::Paste);
    assert_eq!(s.message, "The clipboard is empty.");
    assert_eq!(s.doc.layers().len(), 1);
    assert_eq!(s.doc.undo_count(), 0);
}

#[test]
fn headless_a_failed_write_to_the_os_is_reported_and_keeps_the_app_clipboard() {
    let (mut s, os, _) = painted();
    os.fail_writes(Some(OsClipboardError::Unavailable));
    run(&mut s, ClipAction::Copy);
    assert!(s.clip.pixels.is_some());
    s.poll_clipboard();
    assert!(s.message.contains("OS"), "{}", s.message);
    assert!(os.get().is_none());
    // 貼り付けはアプリの中の写しで通る（OS に画像が無い）
    run(&mut s, ClipAction::Paste);
    assert_eq!(s.doc.layers().len(), 2);
    s.poll_clipboard(); // 知らせは 1 回だけ
    assert!(s.message.contains("ペースト"), "{}", s.message);
}

#[test]
fn headless_copy_merged_takes_the_composite_of_the_paint_channel() {
    let (mut s, _os, id) = painted();
    let top = s.doc.add_layer_above("上", Some(id)).unwrap();
    for y in 20..40 {
        for x in 20..50 {
            s.doc
                .set_pixel(top, x, y, Rgba8::new(255, 0, 0, 128))
                .unwrap();
        }
    }
    s.doc.clear_history().unwrap();
    s.selected_layer = Some(id);
    select(&mut s, 0, 0, 64, 64);
    run(&mut s, ClipAction::CopyMerged);
    let clip = s.clip.pixels.clone().unwrap();
    assert_eq!(clip.source(), ClipboardSource::Composite);
    let composite = s.doc.composite(s.doc.bounds()).unwrap();
    let r = clip.rect();
    for y in 0..clip.height() {
        for x in 0..clip.width() {
            let o = (((r.y + y) * 64 + r.x + x) * 4) as usize;
            assert_eq!(clip.pixel(x, y).unwrap().to_array(), composite[o..o + 4]);
        }
    }
    assert!(s.message.contains("結合してコピー"), "{}", s.message);
    // 選択範囲があっても、合成を写す（層は選ばなくてよい）
    s.selected_layer = None;
    run(&mut s, ClipAction::CopyMerged);
    assert!(s.message.contains("結合してコピー"), "{}", s.message);
}

#[test]
fn headless_mask_editing_copies_and_cuts_the_mask() {
    let (mut s, os, id) = painted();
    s.doc.add_layer_mask(id).unwrap();
    s.doc.set_mask_pixel(id, 12, 12, 200).unwrap();
    select(&mut s, 10, 10, 14, 14);
    s.set_edit_mask(true);
    run(&mut s, ClipAction::Copy);
    let clip = s.clip.pixels.clone().unwrap();
    assert_eq!(clip.source(), ClipboardSource::Mask);
    assert_eq!(
        clip.pixel(2, 2).unwrap(),
        Rgba8::new(55, 55, 55, 255),
        "隠す量 200 は灰色 55"
    );
    assert!(s.message.contains("マスク"), "{}", s.message);
    assert_eq!(os.get().unwrap().width, 4);
    run(&mut s, ClipAction::Cut);
    assert_eq!(
        s.doc
            .layer(id)
            .unwrap()
            .mask()
            .unwrap()
            .surface()
            .pixel(12, 12)
            .unwrap()
            .a,
        0
    );
    assert_eq!(px(&s, id, 12, 12).a, 255, "層の画素はそのまま");
    assert_eq!(s.doc.undo_count(), 1);
    // 貼ると灰色の画素の新しい層になり、マスクを描く状態は終わる
    run(&mut s, ClipAction::Paste);
    assert!(!s.m2.edit_mask);
    assert_eq!(
        px(&s, s.selected_layer.unwrap(), 12, 12),
        Rgba8::new(55, 55, 55, 255)
    );
}

#[test]
fn headless_the_paint_channel_decides_what_is_copied_and_where_it_is_pasted() {
    let (mut s, _os, id) = painted();
    s.doc
        .set_channel_pixel(id, Channel::Roughness, 15, 15, Rgba8::new(90, 90, 90, 255))
        .unwrap();
    s.doc
        .set_channel_pixel(id, Channel::Roughness, 16, 15, Rgba8::new(40, 40, 40, 255))
        .unwrap();
    s.doc.clear_history().unwrap();
    s.m2.paint_channel = Channel::Roughness;
    run(&mut s, ClipAction::Copy);
    let clip = s.clip.pixels.clone().unwrap();
    assert_eq!(clip.channel(), Channel::Roughness);
    assert_eq!((clip.width(), clip.height()), (2, 1));
    run(&mut s, ClipAction::Paste);
    let pasted = s.doc.layer(s.selected_layer.unwrap()).unwrap();
    assert_eq!(
        pasted.surface_channels(),
        vec![Channel::Roughness],
        "今のチャンネルだけを持つ"
    );
    assert!(pasted.is_channel_enabled(Channel::Roughness));
    assert_eq!(
        pasted.pixel(Channel::Roughness, 15, 15).unwrap(),
        Rgba8::new(90, 90, 90, 255)
    );
    // Color の層を Roughness のままカットしようとしても、無効のチャンネルは断る
    s.selected_layer = Some(id);
    s.doc
        .set_channel_enabled(id, Channel::Roughness, false)
        .unwrap();
    run(&mut s, ClipAction::Cut);
    assert!(s.message.contains("無効"), "{}", s.message);
}

// ───────── 断り ─────────

#[test]
fn headless_refusals_say_why_in_both_languages_and_change_nothing() {
    let (mut s, os, id) = painted();
    let group = s.doc.add_group("g", None).unwrap();
    let fill = s
        .doc
        .add_fill_layer("f", &[(Channel::Color, Rgba8::new(1, 2, 3, 255))], None)
        .unwrap();
    let empty = s.doc.add_layer("e").unwrap();
    s.doc.clear_history().unwrap();
    let cases: Vec<(LayerId, ClipAction, &str, &str)> = vec![
        (
            group,
            ClipAction::Copy,
            "この層には画素が無い",
            "This layer has no pixels",
        ),
        (
            group,
            ClipAction::Cut,
            "この層には画素が無い",
            "This layer has no pixels",
        ),
        (
            fill,
            ClipAction::Cut,
            "塗りつぶしの層は切り取れない",
            "A fill layer cannot be cut",
        ),
        (empty, ClipAction::Copy, "層が空", "The layer is empty"),
        (
            LayerId(99),
            ClipAction::Copy,
            "レイヤーが選ばれていません。",
            "No layer is selected.",
        ),
    ];
    for lang in [Lang::Ja, Lang::En] {
        s.lang = lang;
        for (layer, action, ja, en) in &cases {
            s.selected_layer = Some(*layer);
            run(&mut s, *action);
            assert_eq!(s.message, lang.pick(*ja, *en), "{action:?}");
            assert!(
                s.clip.pixels.is_none() && os.get().is_none(),
                "断ったらクリップボードは変えない"
            );
            assert_eq!(s.doc.undo_count(), 0);
        }
    }
    // 画像のロック・透明部分のロック・親グループのロック
    s.lang = Lang::Ja;
    for locks in [
        LayerLocks::PIXELS,
        LayerLocks::TRANSPARENCY,
        LayerLocks::ALL,
    ] {
        s.doc.set_layer_locks(id, locks).unwrap();
        s.doc.clear_history().unwrap();
        s.selected_layer = Some(id);
        run(&mut s, ClipAction::Cut);
        assert!(s.message.contains("ロック"), "{locks:?} {}", s.message);
        assert_eq!(px(&s, id, 12, 15).a, 255);
        assert!(s.clip.pixels.is_none());
        assert_eq!(s.doc.undo_count(), 0);
        run(&mut s, ClipAction::Copy);
        assert!(s.clip.pixels.is_some(), "コピーはロックされていてもできる");
        s.clip.pixels = None;
    }
    s.doc.set_layer_locks(id, LayerLocks::NONE).unwrap();
    s.lang = Lang::En;
    s.doc.set_layer_locks(id, LayerLocks::PIXELS).unwrap();
    run(&mut s, ClipAction::Cut);
    assert_eq!(s.message, "The layer has \"Image pixels\" locked");
    s.doc.set_layer_locks(id, LayerLocks::NONE).unwrap();
    // 写す範囲が上限を超える（バイトや MiB は文に出さない）
    s.doc.clear_history().unwrap();
    s.selected_layer = Some(id);
    s.doc.set_stroke_budget_bytes(1024).unwrap();
    s.lang = Lang::Ja;
    run(&mut s, ClipAction::Copy);
    assert_eq!(s.message, "コピーする範囲が大きすぎる");
    assert!(s.clip.pixels.is_none());
    s.lang = Lang::En;
    run(&mut s, ClipAction::Copy);
    assert_eq!(s.message, "The area to copy is too large");
    // 貼る層が上限を超える（貼る側の話なので、「コピーする範囲」ではなく、貼る層の予算の断りで言う）
    s.doc.set_stroke_budget_bytes(64 << 20).unwrap();
    s.lang = Lang::Ja;
    run(&mut s, ClipAction::Copy);
    s.doc.set_stroke_budget_bytes(1024).unwrap();
    run(&mut s, ClipAction::Paste);
    assert_eq!(s.message, "貼る層が一回の操作の予算を超える");
    s.lang = Lang::En;
    run(&mut s, ClipAction::Paste);
    assert_eq!(s.message, "The pasted layer exceeds the operation budget");
    assert_eq!(s.doc.layers().len(), 4);
    assert_eq!(s.doc.undo_count(), 0);
}

/// 上限は貼る層の実際の大きさ（0 でないタイル）で決まる: 矩形が上限より大きくても、まばらな画像は貼れる。密な画像は断る。
#[test]
fn headless_paste_is_limited_by_the_pasted_layer_not_by_the_size_of_the_rectangle() {
    let mut s = AppState::new(256, 256);
    let os = MemoryClipboard::new();
    s.clip.set_os(Box::new(os.clone()));
    let image = |dense: bool| {
        let mut rgba = vec![0u8; 256 * 256 * 4];
        let opaque = |rgba: &mut Vec<u8>, x: usize, y: usize| {
            let o = (y * 256 + x) * 4;
            rgba[o..o + 4].copy_from_slice(&[(x * 3) as u8, (y * 5) as u8, 9, 255]);
        };
        if dense {
            for y in 0..256 {
                for x in 0..256 {
                    opaque(&mut rgba, x, y);
                }
            }
        } else {
            // 上の行の左端に 4 × 4 だけ
            for y in 0..4 {
                for x in 0..4 {
                    opaque(&mut rgba, x, y);
                }
            }
        }
        ClipImage::new(256, 256, rgba).unwrap()
    };
    let limit = 128 * 1024;
    assert!(
        image(false).rgba.len() as u64 > limit,
        "矩形は上限より大きい"
    );
    s.doc.set_stroke_budget_bytes(limit).unwrap();
    os.set(Some(image(false)));
    run(&mut s, ClipAction::Paste);
    assert_eq!(
        s.doc.layers().len(),
        2,
        "まばらな画像は貼れる: {}",
        s.message
    );
    assert!(s.message.starts_with("ペーストしました"), "{}", s.message);
    os.set(Some(image(true)));
    run(&mut s, ClipAction::Paste);
    assert_eq!(s.doc.layers().len(), 2, "密な画像は断る");
    assert_eq!(s.message, "貼る層が一回の操作の予算を超える");
}

#[test]
fn headless_clipboard_actions_are_refused_while_stroking_and_on_read_only_sets() {
    let (mut s, os, id) = painted();
    select(&mut s, 8, 10, 20, 20);
    run(&mut s, ClipAction::Copy);
    let writes = os.counts().0;
    let stroke = s.begin_canvas_stroke(id, false, None).unwrap();
    for action in [
        ClipAction::Copy,
        ClipAction::Cut,
        ClipAction::CopyMerged,
        ClipAction::Paste,
    ] {
        run(&mut s, action);
        assert!(
            s.message.contains("描いている間"),
            "{action:?} {}",
            s.message
        );
        assert_eq!(s.doc.layers().len(), 1);
        assert_eq!(os.counts().0, writes);
    }
    let entries = menu_entries(&s);
    for action in [
        ClipAction::Copy,
        ClipAction::Cut,
        ClipAction::CopyMerged,
        ClipAction::Paste,
    ] {
        assert!(
            !item(&entries, action).2,
            "{action:?}: 描いている間は使えない"
        );
    }
    s.doc.cancel_stroke(stroke);
    s.doc.clear_history().unwrap();
    // 読むだけのセット
    s.sets.get_mut(0).unwrap().read_only = Some("テスト".into());
    for action in [
        ClipAction::Copy,
        ClipAction::Cut,
        ClipAction::CopyMerged,
        ClipAction::Paste,
    ] {
        run(&mut s, action);
        assert!(s.message.contains("読むだけ"), "{action:?} {}", s.message);
        assert_eq!(s.doc.layers().len(), 1);
        assert_eq!(s.doc.undo_count(), 0);
        assert_eq!(os.counts().0, writes);
    }
    let entries = menu_entries(&s);
    for action in [
        ClipAction::Copy,
        ClipAction::Cut,
        ClipAction::CopyMerged,
        ClipAction::Paste,
    ] {
        assert!(
            !item(&entries, action).2,
            "{action:?}: 読むだけのセットでは使えない"
        );
    }
}

#[test]
fn headless_pasting_a_copy_from_a_bigger_canvas_centres_it_and_reports_the_cut_off_pixels() {
    let (mut s, _os, _) = painted();
    // 128 × 128 の文書から写した写し（64 × 64 の文書へ貼ると、中央に置いて、はみ出した分を切る）
    let mut rgba = Vec::new();
    for _ in 0..100 * 20 {
        rgba.extend([5, 6, 7, 255]);
    }
    let clip = yolu_app::engine::PixelClipboard::new(
        128,
        128,
        10,
        10,
        100,
        20,
        rgba,
        ClipboardSource::Layer,
        Channel::Color,
    )
    .unwrap();
    s.clip.pixels = Some(clip);
    run(&mut s, ClipAction::Paste);
    assert!(
        s.message.contains("中央") && s.message.contains("切り落と"),
        "{}",
        s.message
    );
    let pasted = s.selected_layer.unwrap();
    assert_eq!(px(&s, pasted, 0, 33), Rgba8::new(5, 6, 7, 255));
    s.lang = Lang::En;
    run(&mut s, ClipAction::Paste);
    assert!(
        s.message.contains("centred") && s.message.contains("cut off"),
        "{}",
        s.message
    );
    assert_eq!(s.doc.undo_count(), 2, "どちらも 1 回の Undo");
}

// ───────── OS のクリップボードの状態が怪しいとき ─────────

#[test]
fn headless_pasting_the_app_copy_because_the_os_cannot_be_read_says_why() {
    let (mut s, os, _) = painted();
    run(&mut s, ClipAction::Copy);
    let reasons = [
        (
            OsClipboardError::Busy,
            "OS のクリップボードが使用中です",
            "OS clipboard is busy",
        ),
        (
            OsClipboardError::Unreadable,
            "OS のクリップボードの画像を扱えません",
            "Cannot convert the OS clipboard image",
        ),
        (
            OsClipboardError::Unavailable,
            "OS のクリップボードを使えません",
            "OS clipboard unavailable",
        ),
        (
            OsClipboardError::NotResponding,
            "OS のクリップボードが応答しません",
            "OS clipboard is not responding",
        ),
    ];
    for (error, ja, en) in reasons {
        os.fail_reads(Some(error));
        for lang in [Lang::Ja, Lang::En] {
            s.lang = lang;
            let name = format!(
                "{} {}",
                lang.pick("レイヤー", "Layer"),
                s.doc.layers().len() + 1
            );
            run(&mut s, ClipAction::Paste);
            assert_eq!(
                s.message,
                lang.pick(
                    format!("ペーストしました: {name}（{ja}）"),
                    format!("Pasted as {name} ({en})")
                ),
                "アプリの中の写しは貼り、外の画像が読めなかった理由を添える"
            );
        }
    }
    assert_eq!(s.doc.layers().len(), 9);
    // 読めれば、理由は添えない
    os.fail_reads(None);
    s.lang = Lang::Ja;
    run(&mut s, ClipAction::Paste);
    assert_eq!(s.message, "ペーストしました: レイヤー 10");
}

/// 書き込みに失敗したあと OS に残っている古い外の画像は、コピーしたばかりの画素より優先されない。OS の画像が変われば、外の画像を貼る。
#[test]
fn headless_after_a_failed_write_the_old_image_left_on_the_os_does_not_beat_the_new_copy() {
    let (mut s, os, id) = painted();
    os.set(Some(external(8, 6))); // 外のアプリが前にコピーした画像
    os.fail_writes(Some(OsClipboardError::Unavailable));
    select(&mut s, 8, 10, 20, 20);
    run(&mut s, ClipAction::Copy);
    s.poll_clipboard(); // 書き込みの失敗を受ける（このとき OS にある画像が古い画像）
    assert!(s.message.contains("OS"), "{}", s.message);
    assert_eq!(os.get().unwrap().width, 8, "OS には古い画像が残っている");
    run(&mut s, ClipAction::Paste);
    let pasted = s.selected_layer.unwrap();
    assert_ne!(pasted, id);
    assert_eq!(
        px(&s, pasted, 12, 15),
        px(&s, id, 12, 15),
        "新しくコピーした画素（古い外の画像ではない）"
    );
    assert_eq!(
        px(&s, pasted, 28, 34).a,
        0,
        "古い外の画像の位置には何も無い"
    );
    assert!(!s.message.contains("中央"), "{}", s.message);
    // 何度貼っても同じ
    run(&mut s, ClipAction::Paste);
    assert_eq!(
        px(&s, s.selected_layer.unwrap(), 12, 15),
        px(&s, id, 12, 15)
    );
    // OS の画像が変われば（別のアプリがコピーした）、その外の画像を貼る
    os.set(Some(external(10, 4)));
    run(&mut s, ClipAction::Paste);
    let external_layer = s.selected_layer.unwrap();
    assert_eq!(px(&s, external_layer, 27, 33), Rgba8::new(1, 2, 77, 255));
    assert!(s.message.contains("中央"), "{}", s.message);
    // 古かった画像と同じ画像を、外のアプリがまたコピーしたら、それは外の画像（変わったことを見たので、古い印は外れている）
    os.set(Some(external(8, 6)));
    run(&mut s, ClipAction::Paste);
    assert_eq!(
        px(&s, s.selected_layer.unwrap(), 28, 34),
        Rgba8::new(1, 2, 77, 255)
    );
    // そのあとに OS の画像が消えれば、アプリの中の写しを貼る
    os.set(None);
    run(&mut s, ClipAction::Paste);
    assert_eq!(
        px(&s, s.selected_layer.unwrap(), 12, 15),
        px(&s, id, 12, 15)
    );
    // 書き込みが通るようになってコピーし直せば、自分の画像として貼る
    os.fail_writes(None);
    os.set(Some(external(10, 4)));
    run(&mut s, ClipAction::Copy);
    run(&mut s, ClipAction::Paste);
    assert_eq!(
        px(&s, s.selected_layer.unwrap(), 12, 15),
        px(&s, id, 12, 15)
    );
}

/// 書き込みに失敗したとき OS の画像が読めなかったら、その後に最初に読めた画像を古い画像とみなす。
#[test]
fn headless_when_the_os_image_was_unreadable_at_the_failure_the_next_one_read_is_the_old_one() {
    let (mut s, os, id) = painted();
    os.set(Some(external(8, 6)));
    os.fail_writes(Some(OsClipboardError::Busy));
    os.fail_reads(Some(OsClipboardError::Busy));
    run(&mut s, ClipAction::Copy);
    s.poll_clipboard();
    os.fail_reads(None);
    run(&mut s, ClipAction::Paste);
    assert_eq!(
        px(&s, s.selected_layer.unwrap(), 12, 15),
        px(&s, id, 12, 15)
    );
    assert!(!s.message.contains("中央"), "{}", s.message);
    os.set(Some(external(10, 4)));
    run(&mut s, ClipAction::Paste);
    assert_eq!(
        px(&s, s.selected_layer.unwrap(), 27, 33),
        Rgba8::new(1, 2, 77, 255),
        "違う画像になったので、外の画像"
    );
}

/// 自分が書いた画像が画布より大きいとき（大きな画布から写した写しを小さな画布へ）は、外の画像でなく自分の写しなので、断らず中央に貼る。
/// 同じ大きさの外の画像は断る。
#[test]
fn headless_a_copy_from_a_bigger_canvas_is_pasted_but_an_external_image_of_that_size_is_refused() {
    let os = MemoryClipboard::new();
    let mut big = AppState::new(128, 128);
    big.clip.set_os(Box::new(os.clone()));
    let id = big.selected_layer.unwrap();
    for y in 10..30 {
        for x in 10..110 {
            big.doc
                .set_pixel(id, x, y, Rgba8::new(5, 6, 7, 255))
                .unwrap();
        }
    }
    big.doc.clear_history().unwrap();
    run(&mut big, ClipAction::Copy);
    assert_eq!(os.get().unwrap().width, 100);
    let mut small = AppState::new(64, 64);
    small.clip = std::mem::take(&mut big.clip);
    run(&mut small, ClipAction::Paste);
    assert_eq!(small.doc.layers().len(), 2, "{}", small.message);
    assert!(small.message.contains("切り落と"), "{}", small.message);
    os.set(Some(external(100, 20)));
    run(&mut small, ClipAction::Paste);
    assert_eq!(small.doc.layers().len(), 2);
    assert!(
        small.message.contains("大きい") && small.message.contains("100 × 20"),
        "{}",
        small.message
    );
}

// ───────── 知らせの形 ─────────

/// 知らせは文の形にせず、コピーの知らせと同じく句点・ピリオドで終えない。補足の括弧はその言語のもの。
#[test]
fn headless_clipboard_notices_do_not_end_in_a_full_stop_and_use_the_language_s_brackets() {
    for lang in [Lang::Ja, Lang::En] {
        let (mut s, os, _) = painted();
        s.lang = lang;
        let mut notices = Vec::new();
        fn step(s: &mut AppState, action: ClipAction, notices: &mut Vec<String>) {
            run(s, action);
            notices.push(s.message.clone());
        }
        step(&mut s, ClipAction::Copy, &mut notices);
        step(&mut s, ClipAction::CopyMerged, &mut notices);
        step(&mut s, ClipAction::Cut, &mut notices);
        step(&mut s, ClipAction::Paste, &mut notices);
        let pastes = notices.len() - 1;
        os.set(Some(external(8, 6)));
        step(&mut s, ClipAction::Paste, &mut notices); // 中央
        os.fail_reads(Some(OsClipboardError::Busy));
        step(&mut s, ClipAction::Paste, &mut notices); // OS を読めなかった理由
        os.fail_reads(None);
        // 大きな画布から写した写し: 中央に貼り、はみ出しを切り落とす
        os.set(None);
        let mut rgba = Vec::new();
        for _ in 0..100 * 20 {
            rgba.extend([5, 6, 7, 255]);
        }
        s.clip.pixels = Some(
            yolu_app::engine::PixelClipboard::new(
                128,
                128,
                10,
                10,
                100,
                20,
                rgba,
                ClipboardSource::Layer,
                Channel::Color,
            )
            .unwrap(),
        );
        step(&mut s, ClipAction::Paste, &mut notices);
        for (i, notice) in notices.iter().enumerate() {
            assert!(
                !notice.ends_with(['.', '。', '、', ',']),
                "{lang:?} {i}: {notice}"
            );
        }
        for notice in &notices[pastes..] {
            match lang {
                Lang::Ja => assert!(!notice.contains(['(', ')', '.']), "{notice}"),
                Lang::En => assert!(notice.is_ascii() && !notice.contains('。'), "{notice}"),
            }
        }
        // 補足は 1 つの括弧に並べる
        let last = notices.last().unwrap();
        assert_eq!(last.matches(lang.pick('（', '(')).count(), 1, "{last}");
        assert!(last.ends_with(lang.pick('）', ')')), "{last}");
    }
}

// ───────── メニュー ─────────

#[test]
fn headless_the_edit_menu_entries_follow_the_layer_kind_and_the_mask_state() {
    let (mut s, _os, id) = painted();
    let enabled = |s: &AppState| {
        let e = menu_entries(s);
        [
            ClipAction::Cut,
            ClipAction::Copy,
            ClipAction::CopyMerged,
            ClipAction::Paste,
        ]
        .map(|a| item(&e, a).2)
    };
    assert_eq!(enabled(&s), [true; 4]);
    let group = s.doc.add_group("g", None).unwrap();
    s.selected_layer = Some(group);
    assert_eq!(
        enabled(&s),
        [false, false, true, true],
        "グループは写せない"
    );
    let fill = s
        .doc
        .add_fill_layer("f", &[(Channel::Color, Rgba8::new(1, 2, 3, 255))], None)
        .unwrap();
    s.selected_layer = Some(fill);
    assert_eq!(
        enabled(&s),
        [false, true, true, true],
        "塗りつぶしは写せるが切れない"
    );
    s.selected_layer = Some(group);
    s.doc.add_layer_mask(group).unwrap();
    s.set_edit_mask(true);
    assert_eq!(enabled(&s), [true; 4], "マスクはどの層でも写せて切れる");
    s.set_edit_mask(false);
    s.selected_layer = Some(LayerId(99));
    assert_eq!(enabled(&s), [false, false, true, true]);
    s.selected_layer = Some(id);
    // 名前とキー（日英）
    let e = menu_entries(&s);
    assert_eq!(item(&e, ClipAction::Cut), ("カット", Some("Ctrl+X"), true));
    assert_eq!(item(&e, ClipAction::Copy), ("コピー", Some("Ctrl+C"), true));
    assert_eq!(
        item(&e, ClipAction::CopyMerged),
        ("結合してコピー", Some("Ctrl+Shift+C"), true)
    );
    assert_eq!(
        item(&e, ClipAction::Paste),
        ("ペースト", Some("Ctrl+V"), true)
    );
    s.lang = Lang::En;
    let e = menu_entries(&s);
    assert_eq!(item(&e, ClipAction::Cut).0, "Cut");
    assert_eq!(item(&e, ClipAction::Copy).0, "Copy");
    assert_eq!(item(&e, ClipAction::CopyMerged).0, "Copy Merged");
    assert_eq!(item(&e, ClipAction::Paste).0, "Paste");
}

#[test]
fn edit_menu_lists_the_clipboard_entries_and_runs_them() {
    let mut h = app(1000.0, 640.0, 128);
    let os = MemoryClipboard::new();
    h.state_mut().state.clip.set_os(Box::new(os.clone()));
    let id = h.state().state.selected_layer.unwrap();
    for y in 10..30 {
        for x in 10..30 {
            h.state_mut()
                .state
                .doc
                .set_pixel(id, x, y, Rgba8::new(200, 10, 20, 255))
                .unwrap();
        }
    }
    h.run();
    let title = menu_title(&h, "編集").center();
    click(&mut h, title);
    for label in ["カット", "コピー", "結合してコピー", "ペースト"] {
        popup_item(&h, label);
    }
    let copy = popup_item(&h, "コピー").center();
    click(&mut h, copy);
    assert!(st(&h).popup.is_none(), "選んだら閉じる");
    assert!(st(&h).clip.pixels.is_some());
    assert_eq!(os.counts().0, 1);
    let title = menu_title(&h, "編集").center();
    click(&mut h, title);
    let paste = popup_item(&h, "ペースト").center();
    click(&mut h, paste);
    assert_eq!(st(&h).doc.layers().len(), 2);
    assert_eq!(st(&h).doc.undo_count(), 1);
    // 英語
    h.state_mut().state.lang = Lang::En;
    h.run();
    let title = menu_title(&h, "Edit").center();
    click(&mut h, title);
    for label in ["Cut", "Copy", "Copy Merged", "Paste"] {
        popup_item(&h, label);
    }
    h.snapshot("clipboard_edit_menu");
}

// ───────── キー ─────────

fn layers(h: &H) -> usize {
    st(h).doc.layers().len()
}

#[test]
fn keys_copy_cut_and_paste_in_every_form_the_platform_sends_them() {
    let mut h = app(1000.0, 640.0, 128);
    let os = MemoryClipboard::new();
    h.state_mut().state.clip.set_os(Box::new(os.clone()));
    let id = h.state().state.selected_layer.unwrap();
    for y in 10..30 {
        for x in 10..30 {
            h.state_mut()
                .state
                .doc
                .set_pixel(id, x, y, Rgba8::new(200, 10, 20, 255))
                .unwrap();
        }
    }
    h.state_mut().state.doc.clear_history().unwrap();
    h.run();
    let cmd = Modifiers::COMMAND;
    // Key の押下（egui_kittest・Web など）
    key(&h, Key::C, cmd);
    h.run();
    assert_eq!(
        st(&h).clip.pixels.as_ref().unwrap().source(),
        ClipboardSource::Layer
    );
    key(&h, Key::C, cmd | Modifiers::SHIFT);
    h.run();
    assert_eq!(
        st(&h).clip.pixels.as_ref().unwrap().source(),
        ClipboardSource::Composite
    );
    key(&h, Key::V, cmd);
    h.run();
    assert_eq!(layers(&h), 2, "押して離しても 1 回だけ貼る");
    key(&h, Key::X, cmd);
    h.run();
    assert_eq!(st(&h).doc.undo_count(), 2);
    assert!(st(&h).message.contains("カット"), "{}", st(&h).message);
    assert_eq!(
        st(&h).color.main,
        [0.0, 0.0, 0.0, 1.0],
        "Ctrl+X は色の入れ替え（X）にならない"
    );
    h.state_mut().state.selected_layer = Some(id);

    // egui-winit の置き換え後の事象: Copy・Cut・Paste(文字)
    h.event(Event::Copy);
    h.run();
    assert!(st(&h).message.contains("コピー"), "{}", st(&h).message);
    assert_eq!(
        st(&h).clip.pixels.as_ref().unwrap().source(),
        ClipboardSource::Layer
    );
    // Ctrl+Shift+C は Copy として来る。Shift は事象の並びの中で見る（同じフレームで離しても結合してコピー）
    h.event(Event::ModifiersChanged(cmd | Modifiers::SHIFT));
    h.event(Event::Copy);
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.run();
    assert_eq!(
        st(&h).clip.pixels.as_ref().unwrap().source(),
        ClipboardSource::Composite
    );
    let before = layers(&h);
    h.event(Event::Paste("文字".into()));
    h.run();
    assert_eq!(layers(&h), before + 1);
    // Paste のあとの V の離しは、もう一度は貼らない
    h.event(Event::Key {
        key: Key::V,
        physical_key: None,
        pressed: false,
        repeat: false,
        modifiers: cmd,
    });
    h.run();
    assert_eq!(layers(&h), before + 1);
    // 画像だけのクリップボードでは押下の事象が来ない: Ctrl を押したままの V の離しだけで貼る
    h.event(Event::Key {
        key: Key::V,
        physical_key: None,
        pressed: false,
        repeat: false,
        modifiers: cmd,
    });
    h.run();
    assert_eq!(layers(&h), before + 2);
    // Ctrl の無い V の離しは何もしない
    h.event(Event::Key {
        key: Key::V,
        physical_key: None,
        pressed: false,
        repeat: false,
        modifiers: Modifiers::NONE,
    });
    h.run();
    assert_eq!(layers(&h), before + 2);
}

#[test]
fn clipboard_keys_do_not_fire_while_typing_or_while_a_menu_is_open() {
    let mut h = app(1000.0, 640.0, 128);
    let id = h.state().state.selected_layer.unwrap();
    h.state_mut()
        .state
        .doc
        .set_pixel(id, 5, 5, Rgba8::new(1, 2, 3, 255))
        .unwrap();
    h.state_mut().state.doc.clear_history().unwrap();
    h.run();
    let title = menu_title(&h, "ファイル").center();
    click(&mut h, title);
    key(&h, Key::C, Modifiers::COMMAND);
    h.run();
    assert!(
        st(&h).clip.pixels.is_none(),
        "メニューを開いている間はキーを見ない"
    );
    key(&h, Key::Escape, Modifiers::NONE);
    h.run();
    key(&h, Key::C, Modifiers::COMMAND);
    h.run();
    assert!(st(&h).clip.pixels.is_some());
}

#[test]
fn a_blocked_v_release_clears_the_handled_mark_so_the_next_image_paste_is_not_missed() {
    let mut h = app(1000.0, 640.0, 128);
    let id = h.state().state.selected_layer.unwrap();
    h.state_mut()
        .state
        .doc
        .set_pixel(id, 5, 5, Rgba8::new(1, 2, 3, 255))
        .unwrap();
    h.state_mut().state.doc.clear_history().unwrap();
    h.run();
    key(&h, Key::C, Modifiers::COMMAND);
    h.run();
    let release = || Event::Key {
        key: Key::V,
        physical_key: None,
        pressed: false,
        repeat: false,
        modifiers: Modifiers::COMMAND,
    };
    h.event(Event::Paste("x".into()));
    h.run();
    assert_eq!(layers(&h), 2);
    // メニューを開いた状態での V の離し（見送る印を片づけるだけ）
    let title = menu_title(&h, "ファイル").center();
    click(&mut h, title);
    h.event(release());
    h.run();
    key(&h, Key::Escape, Modifiers::NONE);
    h.run();
    h.event(release());
    h.run();
    assert_eq!(layers(&h), 3, "印が残っていたら、この貼り付けを見逃す");
}
