//! テキストレイヤー（あとから編集できる文字）: レイヤーの Color の画素は文字の値とフォントから描いた結果で、値と画素は 1 回の Undo で一緒に変わる。
//! 手で塗る・画素を動かすのは断り、値を外す（ラスタライズ）と普通のレイヤーになる。フォントは同梱の BIZ UDPGothic（OFL）。

use yolu_core::text::{self, TextAlign, TextFont, TextSettings};
use yolu_core::{
    Affine2D, BrushSettings, CanvasResampling, Channel, CoreError, Document, HistoryKind, LayerId,
    LayerLocks, Resampling, Rgba8,
};

const FONT: &[u8] = include_bytes!("../../../yolu-app/assets/fonts/BIZUDPGothic-Regular.ttf");

fn hello(x: f64, y: f64) -> TextSettings {
    let mut t = TextSettings::new(
        "Hello 文字",
        TextFont::Bundled("biz-udpgothic".into()),
        x,
        y,
    );
    t.size = 24.0;
    t.color = Rgba8::new(200, 30, 60, 255);
    t
}

fn color_bytes(d: &Document, id: LayerId) -> Vec<u8> {
    d.layer(id)
        .unwrap()
        .surface(Channel::Color)
        .unwrap()
        .canvas_bytes()
        .unwrap()
}

fn rendered(d: &Document, t: &TextSettings) -> Vec<u8> {
    text::render(t, FONT, 0, d.width(), d.height(), d.tile_size(), u64::MAX)
        .unwrap()
        .canvas_bytes()
        .unwrap()
}

fn text_doc() -> (Document, LayerId) {
    let mut d = Document::new(160, 96).unwrap();
    let id = d
        .add_text_layer("文字", hello(8.0, 80.0), FONT, None, false)
        .unwrap();
    (d, id)
}

#[test]
fn adding_a_text_layer_draws_the_value_and_is_one_undo() {
    let mut d = Document::new(160, 96).unwrap();
    let before = d.layers().len();
    let t = hello(8.0, 80.0);
    let id = d
        .add_text_layer("文字", t.clone(), FONT, None, false)
        .unwrap();
    let l = d.layer(id).unwrap();
    assert_eq!(l.text(), Some(&t));
    assert_eq!(l.kind(), yolu_core::LayerKind::Raster);
    assert!(l.is_channel_enabled(Channel::Color));
    let bytes = color_bytes(&d, id);
    assert_eq!(bytes, rendered(&d, &t));
    assert!(
        bytes.as_chunks::<4>().0.iter().any(|p| p[3] == 255),
        "文字が描けている"
    );
    assert_eq!(d.history().last(), Some(HistoryKind::AddLayer));
    assert!(d.undo().unwrap());
    assert_eq!(d.layers().len(), before);
    assert!(d.redo().unwrap());
    assert_eq!(d.layer(id).unwrap().text(), Some(&t));
    assert_eq!(color_bytes(&d, id), bytes);
}

#[test]
fn changing_the_value_redraws_and_undo_restores_both() {
    let (mut d, id) = text_doc();
    let old_bytes = color_bytes(&d, id);
    let old = d.layer(id).unwrap().text().unwrap().clone();
    let mut next = old.clone();
    next.text = "二行の\n文".into();
    next.size = 30.0;
    next.align = TextAlign::Center;
    next.x = 80.0;
    d.set_text(id, next.clone(), FONT, false).unwrap();
    assert_eq!(d.history().last(), Some(HistoryKind::Text));
    assert_eq!(d.layer(id).unwrap().text(), Some(&next));
    assert_eq!(color_bytes(&d, id), rendered(&d, &next));
    assert_ne!(color_bytes(&d, id), old_bytes);
    let count = d.undo_count();
    // 同じ値は何もしない
    d.set_text(id, next.clone(), FONT, false).unwrap();
    assert_eq!(d.undo_count(), count);
    assert!(d.undo().unwrap());
    assert_eq!(d.layer(id).unwrap().text(), Some(&old));
    assert_eq!(color_bytes(&d, id), old_bytes);
    assert!(d.redo().unwrap());
    assert_eq!(d.layer(id).unwrap().text(), Some(&next));
    assert_eq!(color_bytes(&d, id), rendered(&d, &next));
}

#[test]
fn moving_and_rotating_change_the_value_not_the_pixels_by_resampling() {
    let (mut d, id) = text_doc();
    let mut moved = d.layer(id).unwrap().text().unwrap().clone();
    moved.x += 13.0;
    moved.y -= 7.0;
    moved.rotation = 30.0;
    d.set_text(id, moved.clone(), FONT, false).unwrap();
    assert_eq!(color_bytes(&d, id), rendered(&d, &moved));
    // 画素を動かす変形は断る（次の描き直しで消える）。何も変えない
    let before = color_bytes(&d, id);
    assert_eq!(
        d.transform_layer(
            id,
            Affine2D::translation(4.0, 0.0),
            Resampling::Bilinear,
            false
        ),
        Err(CoreError::Unsupported("テキストレイヤーには手で描けない"))
    );
    assert_eq!(color_bytes(&d, id), before);
}

#[test]
fn hand_painting_paths_and_disabling_color_are_refused() {
    let (mut d, id) = text_doc();
    let refused = Err(CoreError::Unsupported("テキストレイヤーには手で描けない"));
    assert_eq!(
        d.begin_stroke(id, &BrushSettings::default()).map(|_| ()),
        refused
    );
    let path = yolu_core::paths::CanvasPath {
        id: 1,
        channel: Channel::Color,
        brush: yolu_core::paths::PathBrush(BrushSettings::default()),
        points: vec![yolu_core::paths::CanvasPoint::new(10.0, 10.0, 1.0).unwrap()],
        material: None,
        style: Default::default(),
    };
    assert_eq!(
        d.set_canvas_path(id, path),
        Err(CoreError::Unsupported(
            "テキストレイヤーにはパスを付けられない"
        ))
    );
    assert_eq!(
        d.set_channel_enabled(id, Channel::Color, false),
        Err(CoreError::Unsupported(
            "テキストレイヤーの Color は無効にできない"
        ))
    );
    // テキストレイヤーでないレイヤーの値は変えられない
    let plain = d.add_layer("普通").unwrap();
    assert_eq!(
        d.set_text(plain, hello(0.0, 50.0), FONT, false),
        Err(CoreError::Unsupported("テキストレイヤーではない"))
    );
}

#[test]
fn an_unreadable_font_or_bad_value_changes_nothing() {
    let (mut d, id) = text_doc();
    let layers = d.layers().len();
    let count = d.undo_count();
    let bytes = color_bytes(&d, id);
    let mut next = d.layer(id).unwrap().text().unwrap().clone();
    next.text = "別の文".into();
    assert_eq!(
        d.set_text(id, next.clone(), b"not a font", false),
        Err(CoreError::InvalidArgument("フォントのファイルを読めない"))
    );
    assert!(d
        .add_text_layer("x", next.clone(), b"not a font", None, false)
        .is_err());
    next.size = 0.0;
    assert_eq!(
        d.set_text(id, next, FONT, false),
        Err(CoreError::InvalidArgument("文字のサイズ"))
    );
    assert_eq!(d.layers().len(), layers);
    assert_eq!(d.undo_count(), count);
    assert_eq!(color_bytes(&d, id), bytes);
}

#[test]
fn rasterizing_drops_the_value_keeps_pixels_and_allows_painting() {
    let (mut d, id) = text_doc();
    let bytes = color_bytes(&d, id);
    let t = d.layer(id).unwrap().text().unwrap().clone();
    d.rasterize(id).unwrap();
    assert_eq!(d.history().last(), Some(HistoryKind::Text));
    assert_eq!(d.layer(id).unwrap().text(), None);
    assert_eq!(color_bytes(&d, id), bytes);
    let stroke = d.begin_stroke(id, &BrushSettings::default()).unwrap();
    d.cancel_stroke(stroke);
    d.undo().unwrap();
    assert_eq!(d.layer(id).unwrap().text(), Some(&t));
    assert_eq!(color_bytes(&d, id), bytes);
}

#[test]
fn locks_refuse_rewriting_and_the_position_lock_only_moving() {
    for lock in [
        LayerLocks::PIXELS,
        LayerLocks::TRANSPARENCY,
        LayerLocks::ALL,
    ] {
        let (mut d, id) = text_doc();
        d.set_layer_locks(id, lock).unwrap();
        let mut next = d.layer(id).unwrap().text().unwrap().clone();
        next.text = "別".into();
        assert!(
            matches!(
                d.set_text(id, next, FONT, false),
                Err(CoreError::LayerLocked { .. })
            ),
            "{lock:?}"
        );
    }
    let (mut d, id) = text_doc();
    d.set_layer_locks(id, LayerLocks::POSITION).unwrap();
    let mut next = d.layer(id).unwrap().text().unwrap().clone();
    next.color = Rgba8::new(0, 0, 255, 255);
    d.set_text(id, next.clone(), FONT, false).unwrap();
    next.x += 5.0;
    assert_eq!(
        d.set_text(id, next, FONT, false),
        Err(CoreError::LayerLocked {
            layer: id,
            holder: id,
            lock: LayerLocks::POSITION
        })
    );
}

#[test]
fn merging_bakes_the_text_and_duplicating_keeps_it() {
    let (mut d, id) = text_doc();
    let copy = d.duplicate_layer(id, None).unwrap();
    assert_eq!(d.layer(copy).unwrap().text(), d.layer(id).unwrap().text());
    let report = d
        .merge_down(copy, Document::MERGE_ROUNDING_TOLERANCE)
        .unwrap();
    assert_ne!(report.notes & 16, 0);
    assert_eq!(d.layer(report.result_id).unwrap().text(), None);
}

#[test]
fn resizing_moves_the_value_with_the_pixels() {
    let (mut d, id) = text_doc();
    let t = d.layer(id).unwrap().text().unwrap().clone();
    let report = d.resize_canvas(200, 120, (10, 4)).unwrap();
    let shifted = d.layer(id).unwrap().text().unwrap().clone();
    assert_eq!((shifted.x, shifted.y), (t.x + 10.0, t.y + 4.0));
    assert!(report.notes.is_empty(), "{:?}", report.notes);
    let report = d
        .resize_image(400, 240, CanvasResampling::Bilinear)
        .unwrap();
    let scaled = d.layer(id).unwrap().text().unwrap();
    assert_eq!((scaled.x, scaled.y), (shifted.x * 2.0, shifted.y * 2.0));
    assert_eq!(scaled.size, t.size * 2.0);
    assert!(report.notes.iter().any(|n| n.contains("文字")));
    d.undo().unwrap();
    assert_eq!(d.layer(id).unwrap().text(), Some(&shifted));
}

#[test]
fn smart_materials_keep_only_the_pixels() {
    let (d, id) = text_doc();
    let m = d.capture_smart_material(&[id], "文字の素材").unwrap();
    assert!(m.layers().iter().all(|l| l.text().is_none()));
    assert!(m.notes().iter().any(|n| n.contains("文字")));
}

#[test]
fn loading_attaches_the_value_without_history() {
    let (src, id) = text_doc();
    let t = src.layer(id).unwrap().text().unwrap().clone();
    // 読み込みは画素を描き直さない（フォントが無くても今の画素を見せる）
    let mut d = Document::new(160, 96).unwrap();
    let plain = d.add_layer("読み込み").unwrap();
    d.set_pixel(plain, 3, 3, Rgba8::new(9, 9, 9, 255)).unwrap();
    d.clear_history().unwrap();
    d.set_text_for_load(plain, t.clone()).unwrap();
    assert_eq!(d.undo_count(), 0);
    assert_eq!(d.layer(plain).unwrap().text(), Some(&t));
    assert_eq!(
        d.set_text_for_load(plain, t),
        Err(CoreError::Unsupported(
            "テキストの値を付けられるのはパスとテキストの無いラスターのレイヤーだけ"
        ))
    );
}

#[test]
fn typing_is_coalesced_into_one_undo_step_for_a_new_and_an_existing_layer() {
    // 新しいレイヤー: 追加の段へ、打った分がまとまる
    let mut d = Document::new(160, 96).unwrap();
    let mut t = hello(8.0, 80.0);
    t.text = "H".into();
    let id = d
        .add_text_layer("文字", t.clone(), FONT, None, true)
        .unwrap();
    for text in ["He", "Hel", "Hello"] {
        t.text = text.into();
        d.set_text(id, t.clone(), FONT, true).unwrap();
    }
    d.end_coalescing();
    assert_eq!(d.undo_count(), 1);
    assert_eq!(d.history().last(), Some(HistoryKind::AddLayer));
    assert_eq!(color_bytes(&d, id), rendered(&d, &t));
    d.undo().unwrap();
    assert!(d.layer(id).is_none());
    d.redo().unwrap();
    assert_eq!(d.layer(id).unwrap().text(), Some(&t));
    assert_eq!(color_bytes(&d, id), rendered(&d, &t));
    // 今あるレイヤー: 値の段へまとまり、取り消すと最初の値と画素へ戻る
    let before = color_bytes(&d, id);
    let first = t.clone();
    for text in ["Hello!", "Hello!!", "Hi"] {
        t.text = text.into();
        d.set_text(id, t.clone(), FONT, true).unwrap();
    }
    d.end_coalescing();
    assert_eq!(d.undo_count(), 2);
    assert_eq!(color_bytes(&d, id), rendered(&d, &t));
    d.undo().unwrap();
    assert_eq!(d.layer(id).unwrap().text(), Some(&first));
    assert_eq!(color_bytes(&d, id), before);
    d.redo().unwrap();
    assert_eq!(color_bytes(&d, id), rendered(&d, &t));
    // まとめを終えたあとの変更は別の段
    t.size = 30.0;
    d.set_text(id, t.clone(), FONT, true).unwrap();
    assert_eq!(d.undo_count(), 3);
}
