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

// ───────── 3D ビューの近い色（2D と同じ範囲を、押した面の UV の画素から求める） ─────────

mod in_3d {
    use super::*;
    use yolu_app::region::tools::{self, Where};
    use yolu_app::state::{StrokeSource, Tool};
    use yolu_core::geometry::pick;
    use yolu_core::glam::{Vec2, Vec3};

    const SIZE: u32 = 16;

    fn viewport() -> Rect {
        Rect::from_min_size(pos2(100.0, 100.0), vec2(600.0, 400.0))
    }

    /// 試しの立方体を、右と手前の面が見えるカメラで読んだ文書。近い色のバケツ。
    fn cube() -> AppState {
        let mut s = app();
        s.view3d.load_demo();
        s.view3d.camera.yaw = -40.0;
        s.view3d.camera.pitch = 15.0;
        s.tool = Tool::Fill;
        s.region.tolerance = 0;
        s.color.set_main([0.0, 1.0, 0.0, 1.0]);
        s
    }

    fn screen_of(s: &AppState, p: Vec3) -> egui::Pos2 {
        let view = s
            .view3d
            .camera
            .view(viewport().width(), viewport().height());
        let q = view.to_screen(p).expect("カメラの前");
        pos2(viewport().left() + q.x, viewport().top() + q.y)
    }

    /// 画面の点の下の面の UV が指す画素（今のテクスチャセットの面だけ）。
    fn texel_under(s: &AppState, at: egui::Pos2) -> Option<(u32, u32)> {
        let model = s.view3d.model.as_ref()?;
        let rect = viewport();
        let view = s.view3d.camera.view(rect.width(), rect.height());
        let hit = pick(
            &model.geometry,
            &view,
            Vec2::new(at.x - rect.left(), at.y - rect.top()),
        )?;
        (hit.material == s.view3d.material).then_some((
            (hit.uv.x * SIZE as f32) as u32,
            (hit.uv.y * SIZE as f32) as u32,
        ))
    }

    /// 面の上の候補のうち、画素の左右 2 つ先まで文書の中に収まる点（点・画面の位置・画素）。
    fn spot(s: &AppState) -> (Vec3, egui::Pos2, (u32, u32)) {
        [
            Vec3::new(0.0, 0.0, -0.5),
            Vec3::new(0.2, 0.2, -0.5),
            Vec3::new(-0.2, 0.2, -0.5),
            Vec3::new(0.2, -0.2, -0.5),
            Vec3::new(-0.2, -0.2, -0.5),
            Vec3::new(0.5, 0.2, 0.0),
            Vec3::new(0.5, -0.2, 0.2),
        ]
        .into_iter()
        .find_map(|p| {
            let at = screen_of(s, p);
            let (x, y) = texel_under(s, at)?;
            (3..=12).contains(&x).then_some((p, at, (x, y)))
        })
        .expect("画素が文書の端から離れた面の点がある")
    }

    /// 縦の仕切りを 2 本（画素の列 `x - 2` と `x + 2`）立てた線のレイヤー。仕切りの間の 3 列が、押した画素と同じ近い色の範囲。
    fn walls(s: &mut AppState, x: u32) {
        let id = s.selected_layer.unwrap();
        for column in [x - 2, x + 2] {
            for y in 0..SIZE {
                paint(s, id, column, y, Rgba8::new(0, 0, 0, 255));
            }
        }
        s.doc.clear_history().unwrap();
    }

    fn pixels(s: &AppState) -> Vec<[u8; 4]> {
        (0..SIZE)
            .flat_map(|y| (0..SIZE).map(move |x| (x, y)))
            .map(|(x, y)| yolu_app::engine::composite_pixel(&s.doc, x, y))
            .collect()
    }

    #[test]
    fn a_3d_click_fills_the_same_area_as_a_2d_click_on_the_pixel_under_it() {
        let mut s3 = cube();
        let (_, at, (x, y)) = spot(&s3);
        walls(&mut s3, x);
        // 2D で同じ画素を押した結果
        let mut s2 = cube();
        walls(&mut s2, x);
        let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(160.0, 160.0));
        let view = s2.view.view(rect, SIZE, SIZE);
        let before = pixels(&s3);
        tools::bucket(&mut s3, Where::Surface(viewport()), at);
        tools::bucket(
            &mut s2,
            Where::Canvas(&view),
            view.to_screen(x as f64 + 0.5, y as f64 + 0.5),
        );
        assert_eq!(s3.doc.undo_count(), 1, "1 回の Undo");
        assert_eq!(pixels(&s3), pixels(&s2), "2D と同じ範囲");
        assert_ne!(pixels(&s3), before);
        let green = [0, 255, 0, 255];
        for column in [x - 1, x, x + 1] {
            assert_eq!(yolu_app::engine::composite_pixel(&s3.doc, column, 0), green);
        }
        assert_ne!(yolu_app::engine::composite_pixel(&s3.doc, x - 3, 0), green);
        s3.doc.undo().unwrap();
        assert_eq!(pixels(&s3), before, "1 回の Undo で戻る");
    }

    #[test]
    fn the_tolerance_and_contiguity_of_the_similar_color_apply_in_3d_too() {
        let (mut s3, mut s2) = (cube(), cube());
        let (_, at, (x, y)) = spot(&s3);
        let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(160.0, 160.0));
        for s in [&mut s3, &mut s2] {
            // 押した列だけ少し違う色（許し幅 40 なら同じ範囲、0 なら別）。仕切りの外にも同じ色
            let id = s.selected_layer.unwrap();
            for yy in 0..SIZE {
                for xx in 0..SIZE {
                    let shade = if xx == x { 20 } else { 0 };
                    paint(s, id, xx, yy, Rgba8::new(shade, 0, 0, 255));
                }
            }
            for yy in 0..SIZE {
                paint(s, id, x + 2, yy, Rgba8::new(255, 255, 255, 255));
            }
            s.region.tolerance = 40;
            s.doc.clear_history().unwrap();
        }
        let view = s2.view.view(rect, SIZE, SIZE);
        tools::bucket(&mut s3, Where::Surface(viewport()), at);
        tools::bucket(
            &mut s2,
            Where::Canvas(&view),
            view.to_screen(x as f64 + 0.5, y as f64 + 0.5),
        );
        assert_eq!(pixels(&s3), pixels(&s2));
        // 白い仕切りの向こうは、つながっていないので塗られない
        assert_ne!(
            yolu_app::engine::composite_pixel(&s3.doc, x + 3, 0),
            [0, 255, 0, 255]
        );
        assert_eq!(
            yolu_app::engine::composite_pixel(&s3.doc, x + 1, 0),
            [0, 255, 0, 255]
        );
        // つながりを切ると、仕切りの向こうも同じ色なら塗る（2D と同じ）
        for s in [&mut s3, &mut s2] {
            s.doc.undo().unwrap();
            s.region.contiguous = false;
        }
        tools::bucket(&mut s3, Where::Surface(viewport()), at);
        tools::bucket(
            &mut s2,
            Where::Canvas(&view),
            view.to_screen(x as f64 + 0.5, y as f64 + 0.5),
        );
        assert_eq!(pixels(&s3), pixels(&s2));
        assert_eq!(
            yolu_app::engine::composite_pixel(&s3.doc, x + 3, 0),
            [0, 255, 0, 255]
        );
    }

    #[test]
    fn a_3d_click_with_no_face_of_this_texture_set_under_it_says_why_and_changes_nothing() {
        let mut s = cube();
        // 面の無い所
        let before = pixels(&s);
        tools::bucket(&mut s, Where::Surface(viewport()), pos2(101.0, 101.0));
        assert!(s.message.contains("三角形がありません"), "{}", s.message);
        assert_eq!(s.doc.undo_count(), 0);
        assert_eq!(pixels(&s), before);
        // モデルが無い
        let mut none = app();
        none.tool = Tool::Fill;
        tools::bucket(&mut none, Where::Surface(viewport()), pos2(300.0, 300.0));
        assert_eq!(none.message, "モデルがありません");
        assert_eq!(none.doc.undo_count(), 0);
        // 英語でも
        s.apply(Action::M2Ui(yolu_app::m2::UiOp::Language(
            yolu_app::lang::Lang::En,
        )));
        tools::bucket(&mut s, Where::Surface(viewport()), pos2(101.0, 101.0));
        assert_eq!(
            s.message,
            "No triangle of this texture set under the pointer"
        );
    }

    /// 一周が壁の部屋（外周の 1 画素内側が壁）。部屋の中のどこかに触れた線は、部屋の塗り残しを塗る。
    fn room(s: &mut AppState) {
        let lines = s.doc.add_layer("線").unwrap();
        for i in 1..15 {
            for (x, y) in [(i, 1), (i, 14), (1, i), (14, i)] {
                paint(s, lines, x, y, Rgba8::new(0, 0, 0, 255));
            }
        }
        s.region.color.reference = Reference::Visible;
        s.region.color.leftovers = true;
        s.brush.radius = 0.8;
        s.doc.clear_history().unwrap();
    }

    fn filled(s: &AppState, x: u32, y: u32) -> bool {
        s.doc
            .layer(s.selected_layer.unwrap())
            .unwrap()
            .pixel(Channel::Color, x, y)
            .unwrap()
            .a
            > 0
    }

    #[test]
    fn a_3d_leftover_drag_paints_the_room_it_touched_in_one_undo_and_cancel_throws_it_away() {
        let mut s = cube();
        room(&mut s);
        let (_, at, (x, y)) = spot(&s);
        let rect = viewport();
        // 押した面の UV が部屋の中なら、ドラッグは始まる
        assert!(tools::surface_press(&mut s, rect, at, StrokeSource::Mouse));
        assert!(s.region.leftover_drag.is_some());
        bucket::drag_surface(&mut s, rect, at + vec2(6.0, 2.0));
        // Esc で捨てる: 何も塗らない
        assert!(tools::finish_drag(&mut s, true));
        assert!(s.region.leftover_drag.is_none());
        assert!(!filled(&s, x, y));
        assert_eq!(s.doc.undo_count(), 0);
        // 離す: 部屋が塗られ、1 回の Undo で戻る
        assert!(tools::surface_press(&mut s, rect, at, StrokeSource::Mouse));
        bucket::drag_surface(&mut s, rect, at + vec2(6.0, 2.0));
        assert!(tools::finish_drag(&mut s, false));
        assert!(filled(&s, x, y), "押した画素を含む部屋");
        assert!(filled(&s, 7, 7) && filled(&s, 12, 12), "部屋の中の塗り残し");
        assert!(!filled(&s, 0, 0), "壁の外は塗らない");
        let undo = s.doc.undo_count();
        s.doc.undo().unwrap();
        assert_eq!(s.doc.undo_count(), undo - 1);
        assert!(!filled(&s, 7, 7));
    }

    #[test]
    fn a_3d_leftover_drag_is_refused_off_the_model_and_a_second_one_waits_for_the_first() {
        let mut s = cube();
        room(&mut s);
        let rect = viewport();
        // 面の無い所では始めない
        assert!(!tools::surface_press(
            &mut s,
            rect,
            pos2(101.0, 101.0),
            StrokeSource::Mouse
        ));
        assert!(s.message.contains("三角形がありません"), "{}", s.message);
        assert!(s.region.leftover_drag.is_none());
        // 始めている間は、ほかの押しを断る
        let (_, at, _) = spot(&s);
        assert!(tools::surface_press(&mut s, rect, at, StrokeSource::Mouse));
        assert!(!tools::surface_press(&mut s, rect, at, StrokeSource::Mouse));
        assert!(s.region.leftover_drag.is_some(), "最初のドラッグは続く");
        assert!(tools::finish_drag(&mut s, true));
    }

    /// 大きさ `size` の文書の立方体を、カメラの距離 `distance` で見て、半径 `radius` の塗り残しのドラッグで `from` から `to` へ（面の点）、
    /// マウスの動き 8 回でなぞった点の列。
    fn surface_drag(
        size: u32,
        distance: f32,
        radius: f32,
        from: Vec3,
        to: Vec3,
    ) -> Vec<(f64, f64)> {
        let mut s = AppState::new(size, size);
        s.view3d.load_demo();
        s.view3d.camera.yaw = -40.0;
        s.view3d.camera.pitch = 15.0;
        s.view3d.camera.distance = distance;
        s.tool = Tool::Fill;
        s.region.by_color = true;
        s.region.color.leftovers = true;
        s.brush.radius = radius;
        let rect = viewport();
        let (a, b) = (screen_of(&s, from), screen_of(&s, to));
        assert!(
            tools::surface_press(&mut s, rect, a, StrokeSource::Mouse),
            "{}",
            s.message
        );
        for i in 1..=8 {
            bucket::drag_surface(&mut s, rect, a + (b - a) * (i as f32 / 8.0));
        }
        let points = s
            .region
            .leftover_drag
            .as_ref()
            .expect("ドラッグ")
            .points()
            .to_vec();
        assert!(tools::finish_drag(&mut s, true));
        points
    }

    #[test]
    fn a_leftover_drag_inside_one_face_never_breaks_whatever_the_document_size_and_radius() {
        // 引いて見ていて、画面の 4 点ごとの標本が UV で筆の幅より離れても、1 つの面の中では区切らない（2D はいつも点を線でつなぐ）
        let (from, to) = (Vec3::new(-0.4, 0.1, -0.5), Vec3::new(0.4, 0.1, -0.5));
        for size in [256, 1024, 4096] {
            for distance in [3.0, 8.0] {
                for radius in [1.0, 2.0, 8.0] {
                    let points = surface_drag(size, distance, radius, from, to);
                    let breaks = points.iter().filter(|p| !p.0.is_finite()).count();
                    assert_eq!(
                        breaks, 0,
                        "{size}² 距離 {distance} 半径 {radius}: {points:?}"
                    );
                    assert!(points.len() >= 2, "{points:?}");
                    // 点の列は、なぞった向き（左から右）に UV を進む
                    assert!(points.windows(2).all(|w| w[1] != w[0]));
                }
            }
        }
    }

    #[test]
    fn a_leftover_drag_breaks_where_it_crosses_a_uv_seam() {
        // 手前の面から右の面へ（立方体の辺で UV のアイランドが替わる）: 継ぎ目で区切り、両側の点は残す
        for size in [16, 1024] {
            let points = surface_drag(
                size,
                3.0,
                1.0,
                Vec3::new(0.2, 0.1, -0.5),
                Vec3::new(0.5, 0.1, -0.2),
            );
            let at = points
                .iter()
                .position(|p| !p.0.is_finite())
                .unwrap_or_else(|| panic!("{size}²: 継ぎ目で区切る: {points:?}"));
            assert!(
                at > 0 && at + 1 < points.len(),
                "{size}²: 両側に点がある: {points:?}"
            );
        }
    }

    #[test]
    fn the_leftover_path_is_not_joined_across_a_break() {
        // 3 つの部屋（壁の列 1・5・10・14 と、上下の壁）。左の部屋と右の部屋の点の間を線でつなぐと、間の部屋にも触れる。区切り（NaN の点）を
        // 挟めば、間の部屋には触れない
        let touched = |points: Vec<(f64, f64)>| {
            let mut c = app();
            let lines = c.doc.add_layer("線").unwrap();
            for i in 1..15 {
                paint(&mut c, lines, i, 1, Rgba8::new(0, 0, 0, 255));
                paint(&mut c, lines, i, 14, Rgba8::new(0, 0, 0, 255));
            }
            for x in [1, 5, 10, 14] {
                for y in 1..15 {
                    paint(&mut c, lines, x, y, Rgba8::new(0, 0, 0, 255));
                }
            }
            c.region.color.reference = Reference::Visible;
            c.region.color.leftovers = true;
            c.brush.radius = 0.8;
            let request = bucket::request(&c, c.selected_layer.unwrap(), points);
            color::compute(&c.doc, &request, &AtomicBool::new(false)).unwrap()
        };
        let joined = touched(vec![(3.5, 7.5), (12.5, 7.5)]);
        assert!(joined.amount(3, 7) > 0 && joined.amount(7, 7) > 0 && joined.amount(12, 7) > 0);
        let nan = (f64::NAN, f64::NAN);
        let broken = touched(vec![(3.5, 7.5), nan, (12.5, 7.5)]);
        assert!(
            broken.amount(3, 7) > 0 && broken.amount(12, 7) > 0,
            "区切りの両側の点は触れる"
        );
        assert_eq!(broken.amount(7, 7), 0, "間の部屋は線でつながない");
        // 区切りのあとの 1 点だけでも、その点は触れる
        let after = touched(vec![(3.5, 7.5), nan, (7.5, 7.5)]);
        assert!(after.amount(7, 7) > 0 && after.amount(12, 7) == 0);
    }
}
