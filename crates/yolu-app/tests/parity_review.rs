//! レビューで見つかった、組み合わせの欠落・色選択の内部表記・設定の書き込みの時機。
mod common;
use common::{
    app, canvas_rect, move_to, press, release, render_options, with_render_state_cpu_canvas,
};
use egui::{accesskit::Role, epaint::Shape, vec2, Event, Key, Modifiers, PointerButton};
use egui_kittest::{kittest::Queryable, Harness};
use yolu_app::{
    lang::Lang,
    pen::PenInput,
    prefs::PrefsAction,
    shortcuts::gestures::{bindings, Operation},
    state::{Action, AppState},
    windows::Row,
    YoluApp,
};

/// 見出しの下から次の見出しの前までの（名前, キー）。見出しはキーの列が空の行。
fn section(rows: &[Row], title: &str) -> Vec<(String, String)> {
    let start = rows
        .iter()
        .position(|r| r.left == title && r.right.is_empty())
        .unwrap_or_else(|| panic!("見出しがありません: {title}"));
    rows[start + 1..]
        .iter()
        .take_while(|r| !r.right.is_empty())
        .map(|r| (r.left.clone(), r.right.clone()))
        .collect()
}

#[test]
fn modified_mouse_gestures_are_listed_in_every_view_without_a_filler_column() {
    let command = if cfg!(target_os = "macos") {
        "Cmd"
    } else {
        "Ctrl"
    };
    for lang in Lang::ALL {
        let mut app = AppState::new(32, 32);
        app.lang = lang;
        let rows = yolu_app::shortcuts::rows(&app);
        // 中の列は使わない（操作名の「移動」と取り違える固定の語を置かない）
        assert!(rows.iter().all(|r| r.middle.is_empty()));
        let left = lang.pick("左ボタン", "Left Button");
        let middle = lang.pick("中ボタン", "Middle Button");
        let right = lang.pick("右ボタン", "Right Button");
        let expected = [
            (
                lang.pick("2D ビュー", "2D View"),
                vec![
                    (lang.pick("パン", "Pan"), middle.to_string()),
                    (lang.pick("回転", "Rotate"), format!("Shift+{middle}")),
                    (lang.pick("スポイト", "Eyedropper"), format!("Alt+{left}")),
                    (
                        lang.pick("選択範囲に足す", "Add to Selection"),
                        format!("Shift+{left}"),
                    ),
                    (
                        lang.pick("選択範囲から引く", "Subtract from Selection"),
                        format!("{command}+{left}"),
                    ),
                    (
                        lang.pick("選択範囲と重ねる", "Intersect with Selection"),
                        format!("{command}+Shift+{left}"),
                    ),
                ],
            ),
            (
                lang.pick("3D ビュー", "3D View"),
                vec![
                    (lang.pick("回転", "Orbit"), format!("Alt+{left}")),
                    (lang.pick("パン", "Pan"), format!("Shift+{right}")),
                ],
            ),
            (
                lang.pick("ステンシル", "Stencil"),
                vec![
                    (
                        lang.pick("ステンシルの移動", "Move Stencil"),
                        format!("T+{command}+{left}"),
                    ),
                    (
                        lang.pick("ステンシルの拡縮", "Scale Stencil"),
                        format!("T+Alt+{left}"),
                    ),
                    (
                        lang.pick(
                            "ステンシルの回転を 15° 刻みに",
                            "Snap Stencil Rotation to 15°",
                        ),
                        format!("T+Shift+{left}"),
                    ),
                ],
            ),
        ];
        for (title, wanted) in expected {
            let listed = section(&rows, title);
            let missing: Vec<_> = wanted
                .iter()
                .filter(|(action, keys)| !listed.iter().any(|(a, k)| a == action && k == keys))
                .collect();
            assert!(
                missing.is_empty(),
                "{title} にない組み合わせ: {missing:?}\n{listed:?}"
            );
        }
    }
}

fn settings(lang: Lang) -> Harness<'static, AppState> {
    let mut app = AppState::new(32, 32);
    app.lang = lang;
    app.prefs.open = true;
    let mut ready = false;
    let mut h = Harness::builder()
        .with_size(vec2(900.0, 950.0))
        .build_ui_state(
            move |ui, app| {
                if !ready {
                    YoluApp::setup(ui.ctx());
                    ready = true;
                    ui.ctx().request_repaint();
                    return;
                }
                yolu_app::prefs::show(ui.ctx(), app);
            },
            app,
        );
    h.run();
    h
}

fn drawn_text(h: &Harness<'_, AppState>) -> Vec<String> {
    fn collect(shape: &Shape, texts: &mut Vec<String>) {
        match shape {
            Shape::Vec(shapes) => {
                for shape in shapes {
                    collect(shape, texts);
                }
            }
            Shape::Text(t) => texts.push(t.galley.job.text.clone()),
            _ => {}
        }
    }
    let mut texts = Vec::new();
    for shape in &h.output().shapes {
        collect(&shape.shape, &mut texts);
    }
    texts
}

fn assert_color_text(h: &Harness<'_, AppState>, lang: Lang) {
    let texts = drawn_text(h);
    for text in &texts {
        assert!(
            !["U8", "F", "Selected color", "Alpha"].contains(&text.as_str()),
            "内部表記または固定英語: {text}"
        );
        if lang == Lang::Ja {
            assert!(
                ![
                    "Current color",
                    "Red",
                    "Green",
                    "Blue",
                    "Opacity",
                    "Red intensity",
                    "Green intensity",
                    "Blue intensity",
                    "UV wireframe opacity"
                ]
                .contains(&text.as_str()),
                "日本語の色選択に英語が残っています: {text}"
            );
        }
        if lang == Lang::En {
            assert!(
                !text.chars().any(|c| ('\u{3000}'..='\u{9fff}').contains(&c)),
                "日本語が残っています: {text}"
            );
        }
    }
}

#[test]
fn opened_uv_color_picker_has_no_internal_notation_and_is_localized() {
    for lang in Lang::ALL {
        let mut h = settings(lang);
        let before = h.state().prefs.settings.uv_wireframe_color;
        assert_color_text(&h, lang);
        h.get_by_role(Role::ColorWell).click();
        h.run();
        assert_color_text(&h, lang);
        assert!(
            drawn_text(&h)
                .iter()
                .any(|s| s == lang.pick("不透明度", "Opacity")),
            "開いた色選択に言語に合った不透明度がありません"
        );
        assert_eq!(before, h.state().prefs.settings.uv_wireframe_color);
    }
}

#[test]
fn opened_color_tooltips_follow_language_and_sliders_preserve_other_components() {
    for lang in Lang::ALL {
        let mut h = settings(lang);
        h.ctx
            .all_styles_mut(|style| style.interaction.tooltip_delay = 0.0);
        h.get_by_role(Role::ColorWell).click();
        h.run();
        let preview_tip = lang.pick("選択中の色", "Current color");
        let preview = h.get_by_role_and_label(Role::ColorWell, preview_tip).rect();
        h.event(egui::Event::PointerMoved(preview.center()));
        for _ in 0..8 {
            h.step();
        }
        assert!(drawn_text(&h).iter().any(|text| text == preview_tip));
        assert_color_text(&h, lang);
        for (label, tip) in [
            (
                lang.pick("赤", "Red"),
                lang.pick("赤の強さ", "Red intensity"),
            ),
            (
                lang.pick("緑", "Green"),
                lang.pick("緑の強さ", "Green intensity"),
            ),
            (
                lang.pick("青", "Blue"),
                lang.pick("青の強さ", "Blue intensity"),
            ),
            (
                lang.pick("不透明度", "Opacity"),
                lang.pick("UV ワイヤーフレームの不透明度", "UV wireframe opacity"),
            ),
        ] {
            let rect = h.get_by_role_and_label(Role::Slider, label).rect();
            h.event(egui::Event::PointerMoved(rect.center()));
            for _ in 0..8 {
                h.step();
            }
            assert_color_text(&h, lang);
            assert!(
                drawn_text(&h).iter().any(|text| text == tip),
                "ツールチップがありません: {tip}"
            );
        }
        let before = h.state().prefs.settings.uv_wireframe_color;
        for (label, component) in [
            (lang.pick("不透明度", "Opacity"), 3),
            (lang.pick("赤", "Red"), 0),
        ] {
            let rect = h.get_by_role_and_label(Role::Slider, label).rect();
            let at = egui::pos2(rect.left() + rect.width() * 0.25, rect.bottom() - 3.0);
            let previous = h.state().prefs.settings.uv_wireframe_color;
            h.event(egui::Event::PointerMoved(at));
            h.event(egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            });
            h.event(egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            });
            h.run();
            let after = h.state().prefs.settings.uv_wireframe_color;
            assert_ne!(after[component], previous[component]);
            for i in 0..4 {
                if i != component {
                    assert_eq!(previous[i], after[i]);
                }
            }
            assert_color_text(&h, lang);
        }
        assert_eq!(
            &h.state().prefs.settings.uv_wireframe_color[1..3],
            &before[1..3]
        );
        assert!(!h.state().can_undo());
    }
}

use yolu_app::shortcuts::gestures::Binding;

/// 3D ビューとステンシルの組み合わせを、実際の入力処理へ流す。
fn view_or_stencil_gesture(binding: &Binding) {
    use egui::Rect;
    use yolu_app::{stencil::DragKind, view3d::Nav};
    let ctx = egui::Context::default();
    let mut app = AppState::new(32, 32);
    let rect = Rect::from_min_size(egui::Pos2::ZERO, vec2(400.0, 300.0));
    let at = rect.center();
    if binding.scope == "view3d" {
        app.apply(Action::LoadDemoModel);
    } else {
        app.stencil
            .set_image_rgba("Sample", 1, 1, &[255; 4])
            .unwrap();
    }
    // T は先に保持し、その後に Ctrl / Alt を足す（既存入力の契約）。
    let held = binding.held.map(|key| Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    });
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(rect),
            events: held.into_iter().collect(),
            ..Default::default()
        },
        |ui| {
            if binding.scope == "stencil" {
                yolu_app::stencil::update_keys(ui.ctx(), &mut app);
            }
        },
    );
    output.textures_delta.clear();
    let press = Event::PointerButton {
        pos: at,
        button: binding.button,
        pressed: true,
        modifiers: binding.modifiers,
    };
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(rect),
            events: vec![
                Event::ModifiersChanged(binding.modifiers),
                Event::PointerMoved(at),
                press.clone(),
            ],
            ..Default::default()
        },
        |ui| {
            if binding.scope == "view3d" {
                yolu_app::view3d::input::handle(ui, &mut app, rect, &[], false);
            } else {
                yolu_app::stencil::update_keys(ui.ctx(), &mut app);
                assert!(yolu_app::stencil::handle_event(
                    &mut app,
                    &press,
                    rect,
                    true,
                    binding.modifiers.shift
                ));
            }
        },
    );
    output.textures_delta.clear();
    let found = if binding.scope == "view3d" {
        let (nav, button) = app.view3d.input.nav.expect("3D 操作が始まる");
        assert_eq!(button, binding.button);
        match nav {
            Nav::Orbit => Operation::Orbit,
            Nav::Pan => Operation::Pan,
            Nav::Zoom => Operation::Zoom,
        }
    } else {
        let drag = app.stencil.drag.expect("ステンシルの操作が始まる");
        assert_eq!(drag.button, binding.button);
        match drag.kind {
            DragKind::Move => Operation::MoveStencil,
            DragKind::Scale => Operation::ScaleStencil,
            // Shift を足した回転は、同じ回転の操作（刻みは動かしたときに効く。別の試験）
            DragKind::Rotate if binding.operation == Operation::SnapStencilRotation => {
                Operation::SnapStencilRotation
            }
            DragKind::Rotate => Operation::RotateStencil,
        }
    };
    assert_eq!(found, binding.operation, "{binding:?}");
    assert!(!app.can_undo());
    assert!(!app.is_stroking());
}

/// 2D キャンバスの押し（中ボタンのパン・回転、Alt のスポイト）を、実際のウィンドウの入力へ流す。
fn canvas_gesture(binding: &Binding) {
    let mut h = app(1280.0, 800.0, 256);
    let at = canvas_rect(&h).center();
    h.event(Event::ModifiersChanged(binding.modifiers));
    h.step();
    h.event(Event::PointerMoved(at));
    h.event(Event::PointerButton {
        pos: at,
        button: binding.button,
        pressed: true,
        modifiers: binding.modifiers,
    });
    h.step();
    {
        let s = &h.state().state;
        match binding.operation {
            Operation::Pan => assert!(s.canvas.panning && !s.canvas.middle_rotating, "{binding:?}"),
            Operation::Rotate => {
                assert!(s.canvas.middle_rotating && !s.canvas.panning, "{binding:?}")
            }
            // 描く道具の Alt は、ストロークを始めずにスポイトとして働く
            Operation::Pick => {
                assert!(yolu_app::eyedrop::picks(s, true), "{binding:?}");
                assert!(!yolu_app::eyedrop::picks(s, false));
                assert!(!s.is_stroking() && s.canvas.stroke.is_none(), "{binding:?}");
            }
            other => panic!("2D の組み合わせではない: {other:?}"),
        }
        assert!(!s.can_undo());
    }
    h.event(Event::PointerButton {
        pos: at,
        button: binding.button,
        pressed: false,
        modifiers: binding.modifiers,
    });
    h.step();
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.run();
    let s = &h.state().state;
    assert!(!s.canvas.panning && !s.canvas.middle_rotating);
}

/// 選択範囲の道具の Shift・Ctrl を、実際の選択の入力で確かめる（先に左半分を選び、中ほどの帯をなぞる）。
fn selection_gesture(binding: &Binding) {
    use egui::{Pos2, Rect};
    use yolu_app::{
        selection::{
            canvas::{press, release},
            SelAction, SelEdit,
        },
        state::{StrokeSource, Tool},
    };
    use yolu_core::selection::SelectionCombine;
    let mut app = AppState::new(64, 64);
    app.tool = Tool::SelectRect;
    let view = app
        .view
        .view(Rect::from_min_size(Pos2::ZERO, vec2(256.0, 256.0)), 64, 64);
    app.apply(Action::Sel(SelAction::Edit(SelEdit::Rect {
        x0: 0,
        y0: 0,
        x1: 32,
        y1: 64,
        mode: SelectionCombine::Replace,
    })));
    press(
        &mut app,
        &view,
        view.to_screen(16.0, 8.0),
        StrokeSource::Mouse,
        binding.modifiers,
        0.0,
    );
    release(
        &mut app,
        &view,
        view.to_screen(48.0, 56.0),
        StrokeSource::Mouse,
        binding.modifiers,
    );
    let amount = |x: u32| app.doc.selection().map_or(0, |s| s.amount(x, 30));
    // 左半分だけ・重なり・帯だけの 3 か所
    let found = (amount(4), amount(24), amount(40));
    let wanted = match binding.operation {
        Operation::SelectionAdd => (255, 255, 255),
        Operation::SelectionSubtract => (255, 0, 0),
        Operation::SelectionIntersect => (0, 255, 0),
        other => panic!("選択範囲の組み合わせではない: {other:?}"),
    };
    assert_eq!(found, wanted, "{binding:?}");
}

#[test]
fn every_listed_mouse_gesture_matches_the_real_input_handler() {
    let all = bindings();
    for scope in ["canvas", "selection", "view3d", "stencil"] {
        assert!(
            all.iter().any(|b| b.scope == scope),
            "{scope} の組み合わせがありません"
        );
    }
    for binding in all {
        assert!(matches!(
            binding.button,
            PointerButton::Primary | PointerButton::Middle | PointerButton::Secondary
        ));
        match binding.scope {
            "view3d" | "stencil" => view_or_stencil_gesture(binding),
            "canvas" => canvas_gesture(binding),
            "selection" => selection_gesture(binding),
            other => panic!("一覧にない範囲: {other}"),
        }
    }
}

#[test]
fn the_eyedropper_row_covers_exactly_the_tools_where_alt_picks() {
    use yolu_app::state::Tool;
    let mut app = AppState::new(16, 16);
    let mut picking = Vec::new();
    for tool in Tool::ALL {
        app.tool = tool;
        if yolu_app::eyedrop::picks(&app, true) && !yolu_app::eyedrop::picks(&app, false) {
            picking.push(tool);
        }
    }
    // 一覧の「Alt+左ボタン」の行は、描く道具（ここに挙げた道具）のスポイト。道具の組が替わったら行の書き方を見直す
    assert_eq!(
        picking,
        [Tool::Brush, Tool::Eraser, Tool::Fill, Tool::PolygonFill]
    );
}

#[test]
fn shift_snaps_the_stencil_rotation_to_the_listed_step() {
    use egui::Rect;
    // 一覧の「15°」は実装の刻み
    assert_eq!(yolu_app::stencil::ROTATE_STEP, 15.0);
    let rect = Rect::from_min_size(egui::Pos2::ZERO, vec2(400.0, 300.0));
    let start = rect.center() + vec2(60.0, 0.0);
    for shift in [false, true] {
        let ctx = egui::Context::default();
        let mut app = AppState::new(32, 32);
        app.stencil
            .set_image_rgba("Sample", 1, 1, &[255; 4])
            .unwrap();
        let held = Event::Key {
            key: Key::T,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        };
        let press = Event::PointerButton {
            pos: start,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        };
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(rect),
                events: vec![held, Event::PointerMoved(start), press.clone()],
                ..Default::default()
            },
            |ui| {
                yolu_app::stencil::update_keys(ui.ctx(), &mut app);
                assert!(yolu_app::stencil::handle_event(
                    &mut app, &press, rect, true, false
                ));
            },
        );
        output.textures_delta.clear();
        // 中心の回りに 37° 回す
        let a = 37.0_f32.to_radians();
        let to = rect.center() + vec2(60.0 * a.cos(), 60.0 * a.sin());
        app.stencil.update_drag(to, shift);
        let angle = app.stencil.angle;
        if shift {
            assert!(
                (angle / 15.0 - (angle / 15.0).round()).abs() < 1e-3,
                "{angle}"
            );
        } else {
            assert!((angle - 37.0).abs() < 1.0, "{angle}");
        }
    }
}

// ───────── 設定の書き込みの時機 ─────────

fn app_with_settings(path: &std::path::Path) -> Harness<'static, YoluApp> {
    let path = path.to_path_buf();
    let mut h = Harness::builder()
        .with_size(vec2(1280.0, 800.0))
        .with_pixels_per_point(1.0)
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(120)
        .with_render_options(render_options())
        .wgpu()
        .build_eframe(move |cc| {
            with_render_state_cpu_canvas(
                YoluApp::for_context_with_settings(&cc.egui_ctx, Some(path), PenInput::detached()),
                cc.wgpu_render_state.as_ref(),
            )
        });
    h.run();
    h
}

/// 色・不透明度のスライダーは、ドラッグの間は設定のファイルへ書かず、離したときに 1 回だけ書く（退避の数のスライダーと同じ）。
#[test]
fn dragging_a_uv_color_slider_writes_the_settings_once_on_release() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/parity-review-tests")
        .join(std::process::id().to_string());
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("settings.conf");
    std::fs::write(&path, "language=en\n").unwrap();
    let mut h = app_with_settings(&path);
    h.state_mut().state.apply(Action::Prefs(PrefsAction::Open));
    h.run();
    h.get_by_role_and_label(Role::ColorWell, "UV wireframe color and opacity")
        .click();
    h.run();
    let slider = h.get_by_role_and_label(Role::Slider, "Red").rect();
    let y = slider.bottom() - 3.0;
    let at = |fraction: f32| egui::pos2(slider.left() + slider.width() * fraction, y);
    let red = |h: &Harness<'_, YoluApp>| h.state().state.prefs.settings.uv_wireframe_color[0];
    // 書いたかどうかは、ファイルを見張り用の中身に替えておき、書き換えられたかで見る
    let watch = || std::fs::write(&path, "watch\n").unwrap();
    let untouched = || {
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "watch\n",
            "ドラッグの間は書いてはいけない"
        )
    };
    let default_red = red(&h);
    watch();
    press(&h, at(0.25), PointerButton::Primary);
    h.step();
    assert_ne!(red(&h), default_red, "値は動く");
    assert!(h.state().state.prefs.dragging);
    untouched();
    move_to(&h, at(0.75));
    h.step();
    assert_eq!(red(&h), 191);
    assert!(h.state().state.prefs.dragging);
    untouched();
    h.step();
    untouched();
    // 離すと、そのときの値を 1 回だけ書く（続くフレームでは書き直さない）
    release(&h, at(0.75), PointerButton::Primary);
    h.step();
    h.run();
    assert!(!h.state().state.prefs.dragging);
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "language=en\nuv_wireframe_color=191,217,255,153\n"
    );
    watch();
    h.step();
    h.step();
    untouched();
    let _ = std::fs::remove_dir_all(&dir);
}
