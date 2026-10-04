use egui::{vec2, Event, Modifiers, PointerButton, Pos2};
use egui_kittest::{kittest::Queryable, Harness};
use yolu_app::{
    colorsets::{Edit, Swatch},
    lang::Lang,
    panels::colorsets,
    state::AppState,
    YoluApp,
};

fn panel(width: f32, lang: Lang) -> Harness<'static, AppState> {
    let mut ready = false;
    let mut state = AppState::new(16, 16);
    state.lang = lang;
    let mut h = Harness::builder()
        .with_size(vec2(width, 740.0))
        .build_ui_state(
            move |ui, state| {
                if !ready {
                    YoluApp::setup(ui.ctx());
                    ready = true;
                    ui.ctx().request_repaint();
                    return;
                }
                colorsets::show(ui, state);
            },
            state,
        );
    h.run();
    h
}
fn pointer(
    h: &mut Harness<'_, AppState>,
    at: Pos2,
    button: PointerButton,
    pressed: bool,
    modifiers: Modifiers,
) {
    h.event(Event::ModifiersChanged(modifiers));
    h.event(Event::PointerMoved(at));
    h.event(Event::PointerButton {
        pos: at,
        button,
        pressed,
        modifiers,
    });
    h.step();
}
fn click(h: &mut Harness<'_, AppState>, at: Pos2, button: PointerButton, modifiers: Modifiers) {
    pointer(h, at, button, true, modifiers);
    pointer(h, at, button, false, modifiers);
    h.run();
}
fn swatch_at(h: &Harness<'_, AppState>, i: usize) -> Pos2 {
    h.get_by_label(&h.state().colorsets.palette().colors[i].label())
        .rect()
        .center()
}
#[test]
fn left_alt_and_context_menu_choose_foreground_or_background() {
    for lang in Lang::ALL {
        let mut h = panel(300.0, lang);
        let color = h.state().colorsets.palette().colors[0].rgba;
        let at = swatch_at(&h, 0);
        click(&mut h, at, PointerButton::Primary, Modifiers::NONE);
        assert_eq!(h.state().color.main, color);
        let color = h.state().colorsets.palette().colors[1].rgba;
        let at = swatch_at(&h, 1);
        click(&mut h, at, PointerButton::Primary, Modifiers::ALT);
        assert_eq!(h.state().color.sub, color);
        h.event(Event::ModifiersChanged(Modifiers::NONE));
        let main = h.state().color.main;
        let color = h.state().colorsets.palette().colors[2].rgba;
        let at = swatch_at(&h, 2);
        click(&mut h, at, PointerButton::Secondary, Modifiers::NONE);
        h.get_by_label(lang.pick("サブの色", "Background color"))
            .click();
        h.run();
        assert_eq!(h.state().color.sub, color);
        assert_eq!(h.state().color.main, main);
    }
}
#[test]
fn add_replace_remove_reorder_and_escape_use_real_widgets() {
    let mut h = panel(300.0, Lang::En);
    let c = [0.2, 0.4, 0.6, 0.5];
    h.state_mut().color.set_main(c);
    h.get_by_label("Add Color").click();
    h.run();
    assert_eq!(h.state().colorsets.palette().colors.last().unwrap().rgba, c);
    h.state_mut().color.set_main([0.3; 4]);
    h.get_by_label("Replace").click();
    h.run();
    assert_eq!(
        h.state().colorsets.palette().colors.last().unwrap().rgba,
        [0.3; 4]
    );
    h.get_by_label("Delete").click();
    h.run();
    assert_eq!(h.state().colorsets.palette().colors.len(), 16);
    let original = h.state().colorsets.palette().colors.clone();
    let from = swatch_at(&h, 0);
    let to = swatch_at(&h, 3);
    pointer(&mut h, from, PointerButton::Primary, true, Modifiers::NONE);
    h.event(Event::PointerMoved(to));
    h.step();
    pointer(&mut h, to, PointerButton::Primary, false, Modifiers::NONE);
    h.run();
    assert_eq!(h.state().colorsets.palette().colors[3], original[0]);
    let before = h.state().colorsets.palette().clone();
    let from = swatch_at(&h, 0);
    let to = swatch_at(&h, 2);
    pointer(&mut h, from, PointerButton::Primary, true, Modifiers::NONE);
    h.event(Event::PointerMoved(to));
    h.step();
    h.event(Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    });
    h.step();
    pointer(&mut h, to, PointerButton::Primary, false, Modifiers::NONE);
    h.run();
    assert_eq!(h.state().colorsets.palette(), &before);
}
#[test]
fn set_menu_new_duplicate_delete_and_switch() {
    for lang in Lang::ALL {
        let mut h = panel(300.0, lang);
        h.get_by_label(lang.pick("編集", "Edit")).click();
        h.run();
        h.get_by_label(lang.pick("新規セット", "New Set")).click();
        h.run();
        assert_eq!(h.state().colorsets.entries().len(), 2);
        assert!(h.state().colorsets.palette().colors.is_empty());
        h.get_by_label(lang.pick("色を追加", "Add Color")).click();
        h.run();
        h.get_by_label(lang.pick("編集", "Edit")).click();
        h.run();
        h.get_by_label(lang.pick("セットを複製", "Duplicate Set"))
            .click();
        h.run();
        assert_eq!(h.state().colorsets.entries().len(), 3);
        assert_eq!(h.state().colorsets.palette().colors.len(), 1);
        h.get_by_label(lang.pick("編集", "Edit")).click();
        h.run();
        h.get_by_label(lang.pick("セットを削除", "Delete Set"))
            .click();
        h.run();
        assert_eq!(h.state().colorsets.entries().len(), 2);
        h.get_by_role(egui::accesskit::Role::ComboBox).click();
        h.run();
        h.get_by_label("Yolu").click();
        h.run();
        assert_eq!(h.state().colorsets.active_index(), 0);
    }
}
#[test]
fn grid_reflows_after_resize_and_intermediate_corner_captures_main() {
    let mut h = panel(300.0, Lang::En);
    let wide0 = swatch_at(&h, 0);
    let wide7 = swatch_at(&h, 7);
    assert_eq!(wide0.y, wide7.y);
    h.set_size(vec2(150.0, 900.0));
    h.run();
    let narrow0 = swatch_at(&h, 0);
    let narrow7 = swatch_at(&h, 7);
    assert!(narrow7.y > narrow0.y);
    let c = [0.2, 0.6, 0.8, 0.25];
    h.state_mut().color.set_main(c);
    h.get_by_label("Top left #FF0000FF").click();
    h.run();
    assert_eq!(h.state().colorsets.corners[0], c);
    let expected = yolu_app::colorsets::intermediate(h.state().colorsets.corners, 0.5, 0.5);
    let label = Swatch::new(expected).label();
    h.get_by_label(&label).click();
    h.run();
    assert_eq!(h.state().color.main, expected);
}
#[test]
fn history_has_64_clickable_colors_and_both_languages_have_no_instructions() {
    for lang in Lang::ALL {
        let mut h = panel(320.0, lang);
        for i in 0..64 {
            h.state_mut()
                .color
                .set_main([i as f32 / 100.0, 0.22, 0.37, 1.0]);
            h.state_mut().color.remember();
        }
        h.state_mut()
            .colorsets
            .edit(Edit::Rename("Sample".into()))
            .unwrap();
        h.run();
        let c = h.state().color.recent[63];
        h.get_by_label(&Swatch::new(c).label()).click();
        h.run();
        assert_eq!(h.state().color.main, c);
        assert!(h.query_by_label(lang.pick("履歴", "History")).is_some());
        assert!(h
            .query_by_label(lang.pick("中間色", "Intermediate Colors"))
            .is_some());
        fn texts(s: &egui::epaint::Shape, out: &mut Vec<String>) {
            match s {
                egui::epaint::Shape::Vec(v) => {
                    for s in v {
                        texts(s, out)
                    }
                }
                egui::epaint::Shape::Text(t) => out.push(t.galley.job.text.clone()),
                _ => {}
            }
        }
        let mut labels = vec![];
        for s in &h.output().shapes {
            texts(&s.shape, &mut labels);
        }
        for label in labels {
            assert!(
                !label.contains("クリック") && !label.contains("click"),
                "{label}"
            );
            if lang == Lang::En {
                assert!(
                    !label
                        .chars()
                        .any(|c| matches!(c,'\u{3000}'..='\u{30ff}'|'\u{4e00}'..='\u{9fff}')),
                    "{label}"
                );
            }
        }
    }
}

#[test]
fn app_persists_history_with_the_color_sets_tab_closed() {
    use yolu_app::pen::PenInput;
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/colorsets-tests")
        .join(format!("{}-app", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("settings.conf");
    let create = |path: std::path::PathBuf| {
        Harness::builder()
            .with_size(vec2(1000.0, 800.0))
            .build_eframe(move |cc| {
                YoluApp::for_context_with_settings(&cc.egui_ctx, Some(path), PenInput::detached())
            })
    };
    let mut h = create(path.clone());
    h.run();
    for i in 0..70 {
        h.state_mut()
            .state
            .color
            .set_main([i as f32 / 100.0, 0.17, 0.31, 0.0]);
        h.state_mut().state.color.remember();
    }
    h.run();
    let expected = h.state().state.color.recent.clone();
    assert_eq!(expected.len(), 64);
    assert!(dir.join("colorsets/state.conf").is_file());
    drop(h);
    let mut restored = create(path);
    restored.run();
    assert_eq!(restored.state().state.color.recent, expected);
    assert!(restored
        .state()
        .dock
        .find_tab(&yolu_app::Tab::ColorSets)
        .is_some());
}
