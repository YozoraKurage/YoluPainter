//! バケツの色領域と入力の回帰試験。合成素材はすべて試験内で作る。
use egui::{pos2, vec2, Rect};
use std::sync::atomic::AtomicBool;
use yolu_app::{
    region::{
        bucket,
        color::{self, Distance, Reference},
        RegionAction,
    },
    state::{Action, AppState},
};
use yolu_core::{Channel, CoreError, LayerId, Rgba8, SelectionMask};

fn app() -> AppState {
    let mut app = AppState::new(16, 16);
    app.region.by_color = true;
    app.region.tolerance = 0;
    app
}
fn paint(s: &mut AppState, id: LayerId, x: u32, y: u32, c: Rgba8) {
    s.doc.set_pixel(id, x, y, c).unwrap();
}
fn wall(s: &mut AppState, id: LayerId) {
    for i in 3..13 {
        for (x, y) in [(i, 3), (i, 12), (3, i), (12, i)] {
            paint(s, id, x, y, Rgba8::new(0, 0, 0, 255));
        }
    }
}
fn get(s: &AppState, x: f64, y: f64) -> Result<SelectionMask, CoreError> {
    let req = bucket::request(s, s.selected_layer.unwrap(), vec![(x, y)]);
    color::compute(&s.doc, &req, &AtomicBool::new(false))
}
#[test]
fn references_choose_different_regions_and_multiple_marks() {
    let mut s = app();
    let editing = s.selected_layer.unwrap();
    let lines = s.doc.add_layer("線").unwrap();
    wall(&mut s, lines);
    let divider = s.doc.add_layer("仕切り").unwrap();
    for y in 4..12 {
        paint(&mut s, divider, 8, y, Rgba8::new(0, 0, 0, 255));
    }
    assert_eq!(get(&s, 5.0, 5.0).unwrap().amount(0, 0), 255);
    s.region.color.reference = Reference::Visible;
    let all = get(&s, 5.0, 5.0).unwrap();
    assert_eq!(all.amount(10, 5), 0);
    s.region.color.reference = Reference::Marked;
    s.apply(Action::Region(RegionAction::ReferenceLayer(lines)));
    let marked = get(&s, 5.0, 5.0).unwrap();
    assert_eq!(marked.amount(10, 5), 255);
    assert_eq!(marked.amount(0, 0), 0);
    s.apply(Action::Region(RegionAction::ReferenceLayer(divider)));
    assert_eq!(get(&s, 5.0, 5.0).unwrap(), all);
    assert_eq!(s.selected_layer, Some(editing));
}
#[test]
fn marks_do_not_modify_document_and_are_scoped_to_its_identity() {
    let mut s = app();
    let id = s.selected_layer.unwrap();
    let rev = s.doc.revision();
    s.apply(Action::Region(RegionAction::ReferenceLayer(id)));
    assert_eq!(s.doc.revision(), rev);
    let other = app();
    s.doc = other.doc;
    assert!(bucket::request(&s, id, vec![(1.0, 1.0)]).marked.is_empty());
}
#[test]
fn marked_group_preserves_hidden_ancestor() {
    let mut s = app();
    let id = s.selected_layer.unwrap();
    wall(&mut s, id);
    let group = s.doc.group_layers(&[id], "組").unwrap();
    s.region.references.insert((s.doc.id(), group));
    s.region.color.reference = Reference::Marked;
    assert_eq!(get(&s, 6.0, 6.0).unwrap().amount(0, 0), 0);
    s.doc.set_layer_visible(group, false).unwrap();
    assert_eq!(get(&s, 6.0, 6.0).unwrap().amount(0, 0), 255);
}
#[test]
fn gap_width_boundary_and_source_unchanged() {
    let mut s = app();
    let id = s.selected_layer.unwrap();
    wall(&mut s, id);
    for x in 7..9 {
        paint(&mut s, id, x, 12, Rgba8::TRANSPARENT);
    }
    let rev = s.doc.revision();
    s.region.color.gap = 1;
    assert_eq!(get(&s, 6.0, 6.0).unwrap().amount(0, 0), 255);
    s.region.color.gap = 2;
    assert_eq!(get(&s, 6.0, 6.0).unwrap().amount(0, 0), 0);
    assert_eq!(s.doc.revision(), rev);
}
#[test]
fn area_expands_and_contracts() {
    let mut s = app();
    let id = s.selected_layer.unwrap();
    wall(&mut s, id);
    s.region.color.margin = 1;
    let m = get(&s, 6.0, 6.0).unwrap();
    assert_eq!(m.amount(3, 6), 255);
    assert_eq!(m.amount(2, 6), 0);
    s.region.color.margin = -1;
    let m = get(&s, 6.0, 6.0).unwrap();
    assert_eq!(m.amount(4, 6), 0);
    assert_eq!(m.amount(5, 6), 255);
}
#[test]
fn perceptual_difference_changes_membership_but_alpha_is_independent() {
    let mut s = app();
    let id = s.selected_layer.unwrap();
    paint(&mut s, id, 1, 0, Rgba8::new(40, 0, 0, 0));
    paint(&mut s, id, 2, 0, Rgba8::new(0, 0, 0, 40));
    s.region.tolerance = 25;
    assert_eq!(get(&s, 0.0, 0.0).unwrap().amount(1, 0), 0);
    s.region.color.distance = Distance::Perceptual;
    let m = get(&s, 0.0, 0.0).unwrap();
    assert_eq!(m.amount(1, 0), 255);
    assert_eq!(m.amount(2, 0), 0);
}
#[test]
fn leftovers_require_enclosure_area_and_brush_contact() {
    let mut s = app();
    let target = s.selected_layer.unwrap();
    let lines = s.doc.add_layer("線").unwrap();
    wall(&mut s, lines);
    s.region.color.reference = Reference::Visible;
    s.region.color.leftovers = true;
    s.brush.radius = 1.0;
    assert_eq!(get(&s, 6.5, 6.5).unwrap().amount(10, 10), 255);
    assert!(get(&s, 0.5, 0.5).unwrap().is_empty());
    s.region.color.max_area = 63;
    assert!(get(&s, 6.5, 6.5).unwrap().is_empty());
    s.region.color.max_area = 64;
    paint(&mut s, target, 10, 10, Rgba8::new(255, 0, 0, 255));
    assert_eq!(get(&s, 6.5, 6.5).unwrap().amount(10, 10), 0);
}
#[test]
fn leftovers_interpolate_stroke_and_are_one_undo_or_cancel() {
    let mut s = app();
    let id = s.selected_layer.unwrap();
    let lines = s.doc.add_layer("線").unwrap();
    wall(&mut s, lines);
    s.region.color.reference = Reference::Visible;
    s.region.color.leftovers = true;
    s.brush.radius = 0.8;
    let view = s.view.view(
        Rect::from_min_size(pos2(0.0, 0.0), vec2(160.0, 160.0)),
        16,
        16,
    );
    assert!(bucket::begin(&mut s, &view, view.to_screen(1.5, 6.5)));
    bucket::drag(&mut s, &view, view.to_screen(14.5, 6.5));
    assert!(bucket::finish(&mut s, true));
    assert_eq!(
        s.doc
            .layer(id)
            .unwrap()
            .pixel(Channel::Color, 6, 6)
            .unwrap()
            .a,
        0
    );
    assert!(bucket::begin(&mut s, &view, view.to_screen(1.5, 6.5)));
    bucket::drag(&mut s, &view, view.to_screen(14.5, 6.5));
    bucket::finish(&mut s, false);
    assert!(
        s.doc
            .layer(id)
            .unwrap()
            .pixel(Channel::Color, 6, 6)
            .unwrap()
            .a
            > 0
    );
    s.doc.undo().unwrap();
    assert_eq!(
        s.doc
            .layer(id)
            .unwrap()
            .pixel(Channel::Color, 6, 6)
            .unwrap()
            .a,
        0
    );
    s.doc.redo().unwrap();
    assert!(
        s.doc
            .layer(id)
            .unwrap()
            .pixel(Channel::Color, 6, 6)
            .unwrap()
            .a
            > 0
    );
}
#[test]
fn selection_is_applied_after_expansion_and_fill_undo_restores_pixels() {
    let mut s = app();
    let id = s.selected_layer.unwrap();
    wall(&mut s, id);
    s.region.color.margin = 2;
    s.doc
        .set_selection(Some(SelectionMask::rectangle(&s.doc, 5, 5, 7, 7)))
        .unwrap();
    bucket::start(&mut s, vec![(6.0, 6.0)]);
    assert!(
        s.doc
            .layer(id)
            .unwrap()
            .pixel(Channel::Color, 6, 6)
            .unwrap()
            .a
            > 0
    );
    assert_eq!(
        s.doc
            .layer(id)
            .unwrap()
            .pixel(Channel::Color, 8, 6)
            .unwrap()
            .a,
        0
    );
    s.doc.undo().unwrap();
    assert_eq!(
        s.doc
            .layer(id)
            .unwrap()
            .pixel(Channel::Color, 6, 6)
            .unwrap()
            .a,
        0
    );
    assert!(s.doc.selection().is_some());
}
#[test]
fn budget_and_cancel_publish_no_partial_mask() {
    let s = app();
    let mut req = bucket::request(&s, s.selected_layer.unwrap(), vec![(1.0, 1.0)]);
    req.budget = 1;
    assert_eq!(
        color::compute(&s.doc, &req, &AtomicBool::new(false)).unwrap_err(),
        CoreError::WorkingBudgetExceeded
    );
    req.budget = u64::MAX;
    assert_eq!(
        color::compute(&s.doc, &req, &AtomicBool::new(true)).unwrap_err(),
        CoreError::Cancelled
    );
}
#[test]
fn noncontiguous_mode_includes_separate_matching_regions() {
    let mut s = app();
    let id = s.selected_layer.unwrap();
    wall(&mut s, id);
    s.region.contiguous = false;
    let m = get(&s, 6.0, 6.0).unwrap();
    assert_eq!(m.amount(0, 0), 255);
    assert_eq!(m.amount(3, 6), 0);
}
#[test]
fn locks_types_and_source_budget_refuse_without_history() {
    let mut s = app();
    let id = s.selected_layer.unwrap();
    s.doc
        .set_layer_locks(id, yolu_core::LayerLocks::PIXELS)
        .unwrap();
    s.doc.clear_history().unwrap();
    bucket::start(&mut s, vec![(1.0, 1.0)]);
    assert_eq!(s.doc.undo_count(), 0);
    assert!(!s.modified);
    s.doc
        .set_layer_locks(id, yolu_core::LayerLocks::NONE)
        .unwrap();
    let fill = s.doc.add_fill_layer("塗り", &[], None).unwrap();
    s.selected_layer = Some(fill);
    s.doc.clear_history().unwrap();
    bucket::start(&mut s, vec![(1.0, 1.0)]);
    assert_eq!(s.doc.undo_count(), 0);
    s.selected_layer = Some(id);
    s.doc
        .set_source_budget_bytes(s.doc.allocated_bytes())
        .unwrap();
    bucket::start(&mut s, vec![(1.0, 1.0)]);
    assert_eq!(s.doc.undo_count(), 0);
}
fn wait(s: &mut AppState) {
    let ctx = egui::Context::default();
    let start = std::time::Instant::now();
    while s.region.job.is_some() {
        assert!(start.elapsed().as_secs() < 20, "処理待ちの上限");
        bucket::poll(s, &ctx);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}
#[test]
fn asynchronous_fill_keeps_pressed_color_and_one_undo() {
    let mut s = AppState::new(257, 257);
    s.region.by_color = true;
    s.color.set_main([1.0, 0.0, 0.0, 1.0]);
    let id = s.selected_layer.unwrap();
    let before = s.doc.undo_count();
    bucket::start(&mut s, vec![(1.0, 1.0)]);
    assert!(s.region.job.is_some());
    assert!(s.is_stroking());
    s.color.set_main([0.0, 1.0, 0.0, 1.0]);
    s.brush.opacity = 0.0;
    wait(&mut s);
    assert!(!s.is_stroking());
    assert_eq!(s.doc.undo_count(), before + 1);
    assert_eq!(
        s.doc
            .layer(id)
            .unwrap()
            .pixel(Channel::Color, 0, 0)
            .unwrap(),
        Rgba8::new(255, 0, 0, 255)
    );
    s.doc.undo().unwrap();
    assert_eq!(
        s.doc
            .layer(id)
            .unwrap()
            .pixel(Channel::Color, 0, 0)
            .unwrap()
            .a,
        0
    );
}
#[test]
fn asynchronous_result_is_discarded_after_document_changes() {
    let mut s = AppState::new(257, 257);
    s.region.by_color = true;
    let id = s.selected_layer.unwrap();
    bucket::start(&mut s, vec![(1.0, 1.0)]);
    paint(&mut s, id, 0, 0, Rgba8::new(0, 255, 0, 255));
    let count = s.doc.undo_count();
    wait(&mut s);
    assert_eq!(s.doc.undo_count(), count);
    assert_eq!(
        s.doc
            .layer(id)
            .unwrap()
            .pixel(Channel::Color, 1, 1)
            .unwrap()
            .a,
        0
    );
}
#[test]
fn asynchronous_escape_cancels_without_undo() {
    let mut s = AppState::new(257, 257);
    s.region.by_color = true;
    let before = s.doc.undo_count();
    bucket::start(&mut s, vec![(1.0, 1.0)]);
    let ctx = egui::Context::default();
    let input = egui::RawInput {
        events: vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
        ..Default::default()
    };
    let mut out = ctx.run_ui(input, |ui| bucket::poll(&mut s, ui.ctx()));
    out.textures_delta.clear();
    assert!(s.region.job.is_none());
    assert!(!s.is_stroking());
    assert_eq!(s.doc.undo_count(), before);
}
#[test]
fn bucket_properties_draw_new_labels_in_both_languages() {
    use yolu_app::{lang::Lang, panels::region_props, state::Tool, ui::widgets::Rows, YoluApp};
    fn collect(shape: &egui::epaint::Shape, labels: &mut Vec<String>) {
        match shape {
            egui::epaint::Shape::Vec(shapes) => {
                for s in shapes {
                    collect(s, labels)
                }
            }
            egui::epaint::Shape::Text(t) => labels.push(t.galley.job.text.clone()),
            _ => {}
        }
    }
    for (lang, expected) in [
        (
            Lang::Ja,
            [
                "編集しているレイヤー",
                "参照レイヤー",
                "隙間閉じ",
                "塗り残し部分に塗る",
            ],
        ),
        (
            Lang::En,
            [
                "Editing layer",
                "Reference layers",
                "Close gap",
                "Paint unfilled areas",
            ],
        ),
    ] {
        let mut s = app();
        s.tool = Tool::Fill;
        s.lang = lang;
        let ctx = egui::Context::default();
        YoluApp::setup(&ctx);
        let mut labels = Vec::new();
        for _ in 0..3 {
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(420.0, 1200.0))),
                ..Default::default()
            };
            let mut out = ctx.run_ui(raw, |ui| {
                let ctx = ui.ctx().clone();
                let rect = ui.available_rect_before_wrap();
                let mut rows = Rows::new(rect, rect.top());
                region_props::fill_props(ui, &mut s, &mut rows, &ctx);
            });
            out.textures_delta.clear();
            labels.clear();
            for shape in out.shapes {
                collect(&shape.shape, &mut labels);
            }
        }
        for text in expected {
            assert!(labels.iter().any(|s| s == text), "{text}: {labels:?}");
        }
    }
}
#[test]
fn leftovers_also_fill_white_paper_without_repainting_colored_pixels() {
    let mut s = app();
    let id = s.selected_layer.unwrap();
    for y in 0..16 {
        for x in 0..16 {
            paint(&mut s, id, x, y, Rgba8::new(255, 255, 255, 255));
        }
    }
    wall(&mut s, id);
    paint(&mut s, id, 10, 10, Rgba8::new(255, 0, 0, 255));
    s.region.color.leftovers = true;
    let m = get(&s, 6.5, 6.5).unwrap();
    assert_eq!(m.amount(6, 6), 255);
    assert_eq!(m.amount(10, 10), 0);
}
#[test]
fn missing_reference_marks_refuse_in_both_languages() {
    for lang in [yolu_app::lang::Lang::Ja, yolu_app::lang::Lang::En] {
        let mut s = app();
        s.lang = lang;
        s.region.color.reference = Reference::Marked;
        let before = s.doc.undo_count();
        bucket::start(&mut s, vec![(1.0, 1.0)]);
        assert_eq!(s.doc.undo_count(), before);
        assert_eq!(
            s.message,
            lang.pick("参照レイヤーがありません", "No reference layers")
        );
    }
}

#[test]
fn review_marked_clipped_lines_keep_the_unmarked_base() {
    let mut s = app();
    let base = s
        .doc
        .add_fill_layer(
            "下地",
            &[(Channel::Color, Rgba8::new(255, 255, 255, 255))],
            None,
        )
        .unwrap();
    let lines = s.doc.add_layer("線").unwrap();
    wall(&mut s, lines);
    s.doc.set_layer_clipping(lines, true).unwrap();
    s.region.color.reference = Reference::Marked;
    s.region.references.insert((s.doc.id(), lines));
    assert!(!s.region.references.contains(&(s.doc.id(), base)));
    let m = get(&s, 6.0, 6.0).unwrap();
    assert_eq!(m.amount(6, 6), 255);
    assert_eq!(
        m.amount(0, 0),
        0,
        "印のない下地にクリップした線も境界になる"
    );
    assert!(s.doc.layer(base).unwrap().visible());
}

#[test]
fn review_a_second_leftover_stroke_is_refused_until_the_first_finishes() {
    let mut s = AppState::new(64, 64);
    s.region.by_color = true;
    s.region.color.leftovers = true;
    let id = s.selected_layer.unwrap();
    let lines = s.doc.add_layer("線").unwrap();
    wall(&mut s, lines);
    for i in 3..13 {
        for (x, y) in [(i + 20, 3), (i + 20, 12), (23, i), (32, i)] {
            paint(&mut s, lines, x, y, Rgba8::new(0, 0, 0, 255));
        }
    }
    s.doc.clear_history().unwrap();
    s.region.color.reference = Reference::Visible;
    let view = s.view.view(
        Rect::from_min_size(pos2(0.0, 0.0), vec2(640.0, 640.0)),
        64,
        64,
    );
    let p = view.to_screen(6.5, 6.5);
    assert!(bucket::begin(&mut s, &view, p));
    assert!(bucket::finish(&mut s, false));
    assert!(s.region.job.is_some());
    let next = view.to_screen(26.5, 6.5);
    for _ in 0..2 {
        assert!(
            !bucket::begin(&mut s, &view, next),
            "前の処理中に新しいドラッグを受け付けない"
        );
        assert!(s.region.leftover_drag.is_none());
        assert_eq!(s.message, "塗りつぶし中");
    }
    wait(&mut s);
    assert!(
        s.doc
            .layer(id)
            .unwrap()
            .pixel(Channel::Color, 6, 6)
            .unwrap()
            .a
            > 0
    );
    assert_eq!(s.doc.undo_count(), 1);
    assert_eq!(
        s.doc
            .layer(id)
            .unwrap()
            .pixel(Channel::Color, 26, 6)
            .unwrap()
            .a,
        0
    );
    assert!(bucket::begin(&mut s, &view, next), "完了後は次を受け付ける");
    bucket::finish(&mut s, false);
    wait(&mut s);
    assert_eq!(s.doc.undo_count(), 2);
    assert!(
        s.doc
            .layer(id)
            .unwrap()
            .pixel(Channel::Color, 26, 6)
            .unwrap()
            .a
            > 0
    );
    s.doc.undo().unwrap();
    assert_eq!(
        s.doc
            .layer(id)
            .unwrap()
            .pixel(Channel::Color, 26, 6)
            .unwrap()
            .a,
        0
    );
    assert!(
        s.doc
            .layer(id)
            .unwrap()
            .pixel(Channel::Color, 6, 6)
            .unwrap()
            .a
            > 0
    );
}

fn focus_loss_commits(size: u32) {
    use yolu_app::{
        canvas::{self, display::CanvasDisplay},
        state::{StrokeSource, Tool},
        YoluApp,
    };
    let mut s = AppState::new(size, size);
    s.tool = Tool::Fill;
    s.region.by_color = true;
    s.region.color.leftovers = true;
    s.region.color.reference = Reference::Visible;
    let id = s.selected_layer.unwrap();
    let lines = s.doc.add_layer("線").unwrap();
    wall(&mut s, lines);
    s.doc.clear_history().unwrap();
    let ctx = egui::Context::default();
    YoluApp::setup(&ctx);
    let mut display = CanvasDisplay::default();
    let view = s.view.view(
        Rect::from_min_size(pos2(0.0, 0.0), vec2(320.0, 320.0)),
        size,
        size,
    );
    assert!(bucket::begin(&mut s, &view, view.to_screen(6.5, 6.5)));
    s.canvas.stroke = Some(StrokeSource::Mouse);
    let deadline = std::time::Instant::now();
    let mut first = true;
    loop {
        let mut raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(320.0, 320.0))),
            focused: false,
            ..Default::default()
        };
        raw.viewports
            .get_mut(&egui::ViewportId::ROOT)
            .unwrap()
            .focused = Some(false);
        if first {
            raw.events.push(egui::Event::WindowFocused(false));
        }
        let mut out = ctx.run_ui(raw, |ui| {
            bucket::poll(&mut s, ui.ctx());
            canvas::show(ui, &mut s, &mut display, &[]);
        });
        out.textures_delta.clear();
        first = false;
        if s.region.job.is_none() {
            break;
        }
        assert!(deadline.elapsed().as_secs() < 10);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(s.region.leftover_drag.is_none());
    assert!(s.canvas.stroke.is_none());
    assert_eq!(
        s.doc.undo_count(),
        1,
        "サイズ {size}: フォーカスを失ってもそこまでを確定"
    );
    assert!(
        s.doc
            .layer(id)
            .unwrap()
            .pixel(Channel::Color, 6, 6)
            .unwrap()
            .a
            > 0
    );
    s.doc.undo().unwrap();
    assert_eq!(
        s.doc
            .layer(id)
            .unwrap()
            .pixel(Channel::Color, 6, 6)
            .unwrap()
            .a,
        0
    );
    drop(display);
    let mut out = ctx.run_ui(egui::RawInput::default(), |_| {});
    out.textures_delta.clear();
}
#[test]
fn review_focus_loss_commits_a_small_leftover_stroke() {
    focus_loss_commits(16);
}
#[test]
fn review_focus_loss_commits_a_large_leftover_stroke() {
    focus_loss_commits(64);
}

#[test]
fn review_saved_bucket_pixels_survive_but_session_marks_do_not() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/bucket-review-tests")
        .join(std::process::id().to_string());
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("bucket.ylp");
    let mut s = app();
    let id = s.selected_layer.unwrap();
    let lines = s.doc.add_layer("線").unwrap();
    wall(&mut s, lines);
    paint(&mut s, lines, 8, 12, Rgba8::TRANSPARENT);
    s.region.references.insert((s.doc.id(), lines));
    s.region.color.reference = Reference::Marked;
    s.region.color.gap = 1;
    s.region.color.margin = 1;
    s.region.color.distance = Distance::Perceptual;
    s.region.color.leftovers = true;
    s.color.set_main([1.0, 0.0, 0.0, 1.0]);
    bucket::start(&mut s, vec![(6.5, 6.5)]);
    assert_eq!(
        s.doc
            .layer(id)
            .unwrap()
            .pixel(Channel::Color, 6, 6)
            .unwrap(),
        Rgba8::new(255, 0, 0, 255)
    );
    let before = s.doc.composite(yolu_core::Rect::new(0, 0, 16, 16)).unwrap();
    s.apply(Action::SaveProjectAs(path.clone()));
    assert!(s.message.starts_with("保存しました"), "{}", s.message);
    let mut opened = AppState::new(8, 8);
    opened.apply(Action::OpenProject(path));
    assert!(
        opened.message.starts_with("開きました"),
        "{}",
        opened.message
    );
    assert_eq!(
        opened
            .doc
            .composite(yolu_core::Rect::new(0, 0, 16, 16))
            .unwrap(),
        before
    );
    for y in 0..16 {
        for x in 0..16 {
            assert_eq!(
                opened
                    .doc
                    .layer(id)
                    .unwrap()
                    .pixel(Channel::Color, x, y)
                    .unwrap(),
                s.doc
                    .layer(id)
                    .unwrap()
                    .pixel(Channel::Color, x, y)
                    .unwrap()
            );
        }
    }
    assert!(opened.region.references.is_empty());
}

#[test]
fn review_clipping_dependencies_keep_groups_but_not_unmarked_clips() {
    for clipped_parent in [false, true] {
        let mut s = app();
        let base = s
            .doc
            .add_fill_layer(
                "下地",
                &[(Channel::Color, Rgba8::new(255, 255, 255, 255))],
                None,
            )
            .unwrap();
        let base_group = s.doc.group_layers(&[base], "下地の組").unwrap();
        let other = s.doc.add_layer("別の線").unwrap();
        for y in 4..12 {
            paint(&mut s, other, 8, y, Rgba8::new(0, 0, 0, 255));
        }
        s.doc.set_layer_clipping(other, true).unwrap();
        let lines = s.doc.add_layer("参照の線").unwrap();
        wall(&mut s, lines);
        let clipped = if clipped_parent {
            s.doc.group_layers(&[lines], "線の組").unwrap()
        } else {
            lines
        };
        s.doc.set_layer_clipping(clipped, true).unwrap();
        s.region.color.reference = Reference::Marked;
        s.region.references.insert((s.doc.id(), lines));
        let m = get(&s, 6.0, 6.0).unwrap();
        assert_eq!(m.amount(0, 0), 0, "下地グループの子孫を残す");
        assert_eq!(
            m.amount(10, 6),
            255,
            "印のない別のクリッピング線は参照しない"
        );
        s.doc.set_layer_visible(base_group, false).unwrap();
        assert_eq!(
            get(&s, 6.0, 6.0).unwrap().amount(0, 0),
            255,
            "下地を勝手に再表示しない"
        );
    }
}
