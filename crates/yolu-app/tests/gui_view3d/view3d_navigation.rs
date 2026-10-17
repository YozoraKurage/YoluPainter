//! 視点の中心を、実際の入力関数へのマウス・ペンの差し込みと投影座標で検証する。
use crate::common;
use crate::common::drive::Drive;
use egui::{pos2, vec2, Event, Key, Modifiers, PointerButton, Pos2, Rect};
use egui_kittest::{kittest::Queryable, Harness};
use yolu_app::{
    lang::Lang,
    pen::PenSample,
    state::AppState,
    view3d::{
        self,
        navigation::{self, OrbitCenter, Preferences, ZoomCenter},
    },
    YoluApp,
};
use yolu_core::{
    geometry::{pick, OrbitCamera},
    glam::{Vec2, Vec3},
};

struct Fixture {
    app: AppState,
    pen: Vec<PenSample>,
}

fn rect() -> Rect {
    Rect::from_min_size(Pos2::ZERO, vec2(640.0, 480.0))
}

fn harness(preferences: Preferences) -> Harness<'static, Fixture> {
    let mut app = AppState::new(32, 32);
    app.view3d_mut().load_demo();
    app.view3d_mut().camera = OrbitCamera {
        yaw: 0.0,
        pitch: 0.0,
        distance: 3.0,
        ..app.view3d().camera
    };
    app.prefs_mut().settings.navigation = preferences;
    let mut h = common::gpu_thread::builder()
        .with_size(rect().size())
        .build_ui_state(
            |ui, f: &mut Fixture| {
                yolu_app::stencil::update_keys(ui.ctx(), &mut f.app);
                let samples = std::mem::take(&mut f.pen);
                view3d::input::handle(ui, &mut f.app, rect(), &samples, false);
            },
            Fixture {
                app,
                pen: Vec::new(),
            },
        );
    h.run();
    h
}

impl Drive for Harness<'_, Fixture> {
    fn app(&self) -> &AppState {
        &self.state().app
    }
    fn app_mut(&mut self) -> &mut AppState {
        &mut self.state_mut().app
    }
}

fn camera(h: &Harness<'_, Fixture>) -> OrbitCamera {
    h.view3d().camera
}
fn project(c: OrbitCamera, p: Vec3) -> Vec2 {
    c.view(640.0, 480.0).to_screen(p).unwrap()
}
fn hit(h: &Harness<'_, Fixture>, at: Pos2) -> Vec3 {
    pick(
        &h.view3d().model.as_ref().unwrap().geometry,
        &camera(h).view(640.0, 480.0),
        Vec2::new(at.x, at.y),
    )
    .unwrap()
    .position
}
fn near(a: Vec2, b: Vec2) {
    assert!((a - b).length() < 0.02, "{a} != {b}");
}
fn mouse(
    h: &mut Harness<'_, Fixture>,
    at: Pos2,
    button: PointerButton,
    down: bool,
    modifiers: Modifiers,
) {
    h.event(Event::PointerMoved(at));
    h.event(Event::PointerButton {
        pos: at,
        button,
        pressed: down,
        modifiers,
    });
    h.step();
}
fn pen(h: &mut Harness<'_, Fixture>, at: Pos2, contact: bool, barrel: bool) {
    h.state_mut().pen.push(PenSample {
        pos: [at.x, at.y],
        pressure: 1.0,
        tilt: Default::default(),
        rotation: None,
        contact,
        eraser: false,
        barrel,
        pointer_id: 8,
        time_ms: 0,
    });
    h.step();
}
fn key(h: &mut Harness<'_, Fixture>, k: Key, pressed: bool, modifiers: Modifiers) {
    h.event(Event::ModifiersChanged(modifiers));
    h.event(Event::Key {
        key: k,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers,
    });
    h.step();
}
fn prefs(orbit: OrbitCenter, zoom: ZoomCenter) -> Preferences {
    Preferences {
        orbit,
        zoom,
        ..Preferences::default()
    }
}

#[test]
fn surface_orbit_keeps_the_pressed_point_and_mouse_matches_pen() {
    let mut results = Vec::new();
    for use_pen in [false, true] {
        for alt in [false, true] {
            let mut h = harness(prefs(OrbitCenter::Surface, ZoomCenter::View));
            let at = pos2(375.0, 215.0);
            let point = hit(&h, at);
            let mods = if alt { Modifiers::ALT } else { Modifiers::NONE };
            h.event(Event::ModifiersChanged(mods));
            // スナップ回転は Alt + 右（ペンはサイドボタン）。Alt の有無で、同じ右ボタンの回転か切り替えかが決まる
            let button = PointerButton::Secondary;
            if use_pen {
                pen(&mut h, at, true, true);
            } else {
                mouse(&mut h, at, button, true, mods);
            }
            for d in [vec2(30.0, 15.0), vec2(90.0, 55.0)] {
                if use_pen {
                    pen(&mut h, at + d, true, true);
                } else {
                    h.event(Event::PointerMoved(at + d));
                    h.step();
                }
                near(project(camera(&h), point), Vec2::new(at.x, at.y));
            }
            if use_pen {
                pen(&mut h, at + vec2(90.0, 55.0), false, true);
            } else {
                mouse(&mut h, at + vec2(90.0, 55.0), button, false, mods);
            }
            assert!(h.view3d().input.navigation.is_none());
            assert!(!h.doc().can_undo());
            results.push(camera(&h));
        }
    }
    // results: [マウスの右, マウスの Alt + 右, ペンのサイドボタン, ペンのサイドボタン + Alt]。マウスとペンは同じ。Alt + 右は、右へ動かした向きの隣の軸の視点
    for (mouse_result, pen_result) in [(results[0], results[2]), (results[1], results[3])] {
        assert_eq!(
            (pen_result.yaw, pen_result.pitch, pen_result.distance),
            (mouse_result.yaw, mouse_result.pitch, mouse_result.distance)
        );
        assert!((pen_result.target - mouse_result.target).length() < 1e-6);
    }
    assert_eq!((results[1].yaw, results[1].pitch), (90.0, 0.0));
    assert_ne!((results[0].yaw, results[0].pitch), (90.0, 0.0));
}

#[test]
fn snap_orbit_keeps_the_pressed_surface_point_in_place_when_it_flicks_to_the_next_axis_view() {
    // 面の位置を回転の中心にしていても、Alt + ドラッグのスナップ回転は押した面の点を画面の同じ所に保つ（切り替えた向きでも）
    let mut h = harness(prefs(OrbitCenter::Surface, ZoomCenter::View));
    h.view3d_mut().camera.yaw = 25.0;
    let at = pos2(375.0, 215.0);
    let point = hit(&h, at);
    let alt = Modifiers::ALT;
    let d = navigation::FLICK_DISTANCE;
    h.event(Event::ModifiersChanged(alt));
    mouse(&mut h, at, PointerButton::Secondary, true, alt);
    assert!(h.view3d().input.navigation.is_some());
    // 閾値の手前では動かない
    h.event(Event::PointerMoved(at + vec2(-(d - 1.0), 0.0)));
    h.step();
    assert_eq!(camera(&h).yaw, 25.0);
    // 背面（yaw 25° に近い）から左へ: 右の軸の視点（yaw -90°）
    h.event(Event::PointerMoved(at + vec2(-(d + 1.0), 0.0)));
    h.step();
    let c = camera(&h);
    assert_eq!((c.yaw, c.pitch), (-90.0, 0.0));
    near(project(c, point), Vec2::new(at.x, at.y));
    // 続けて左へ: 正面（yaw -180°）。さらに下へ: 上の視点
    h.event(Event::PointerMoved(at + vec2(-2.0 * d - 2.0, 0.0)));
    h.step();
    let c = camera(&h);
    assert_eq!((c.yaw, c.pitch), (-180.0, 0.0));
    near(project(c, point), Vec2::new(at.x, at.y));
    h.event(Event::PointerMoved(at + vec2(-2.0 * d - 2.0, d + 1.0)));
    h.step();
    let c = camera(&h);
    assert_eq!((c.yaw, c.pitch), (-180.0, 90.0));
    near(project(c, point), Vec2::new(at.x, at.y));
    mouse(
        &mut h,
        at + vec2(-2.0 * d - 2.0, d + 1.0),
        PointerButton::Secondary,
        false,
        alt,
    );
    assert!(h.view3d().input.navigation.is_none());
    assert!(!h.doc().can_undo());
}

#[test]
fn all_centers_and_empty_surface_fallback() {
    for mode in OrbitCenter::ALL {
        let mut h = harness(prefs(mode, ZoomCenter::View));
        h.view3d_mut().camera.target = Vec3::new(0.2, 0.0, 0.0);
        let at = pos2(4.0, 4.0); // 面が無い
        let before = camera(&h);
        let pivot = match mode {
            OrbitCenter::View | OrbitCenter::Surface => before.target,
            _ => Vec3::ZERO,
        };
        let screen = project(before, pivot);
        mouse(&mut h, at, PointerButton::Secondary, true, Modifiers::NONE);
        h.event(Event::PointerMoved(at + vec2(30.0, 20.0)));
        h.step();
        near(project(camera(&h), pivot), screen);
        assert!((camera(&h).yaw - before.yaw - 10.5).abs() < 0.001);
    }
}

#[test]
fn auto_depth_pan_tracks_the_pointer_for_middle_space_and_the_pen_with_space() {
    for route in 0..3 {
        let mut h = harness(prefs(OrbitCenter::Surface, ZoomCenter::View));
        let at = pos2(375.0, 215.0);
        let point = hit(&h, at);
        if route == 1 || route == 2 {
            key(&mut h, Key::Space, true, Modifiers::NONE);
        }
        if route == 2 {
            pen(&mut h, at, true, false);
        } else {
            mouse(
                &mut h,
                at,
                if route == 0 {
                    PointerButton::Middle
                } else {
                    PointerButton::Primary
                },
                true,
                Modifiers::NONE,
            );
        }
        let end = at + vec2(45.0, -30.0);
        if route == 2 {
            pen(&mut h, end, true, false);
        } else {
            h.event(Event::PointerMoved(end));
            h.step();
        }
        near(project(camera(&h), point), Vec2::new(end.x, end.y));
    }
}

#[test]
fn pointer_zoom_holds_surface_or_background_for_wheel_drag_and_click() {
    for at in [pos2(375.0, 215.0), pos2(30.0, 40.0)] {
        for route in 0..5 {
            let mut h = harness(prefs(OrbitCenter::View, ZoomCenter::Pointer));
            let c = camera(&h);
            let view = c.view(640.0, 480.0);
            let ray = view.ray(Vec2::new(at.x, at.y));
            let point = pick(
                &h.view3d().model.as_ref().unwrap().geometry,
                &view,
                Vec2::new(at.x, at.y),
            )
            .map(|p| p.position)
            .unwrap_or(
                view.position + ray.direction() * (c.distance / ray.direction().dot(view.forward)),
            );
            if route == 0 {
                h.event(Event::PointerMoved(at));
                h.event(Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Line,
                    delta: vec2(0.0, 1.0),
                    modifiers: Modifiers::NONE,
                    phase: egui::TouchPhase::Move,
                });
                h.step();
            } else {
                key(&mut h, Key::Space, true, Modifiers::CTRL);
                let use_pen = route >= 3;
                if use_pen {
                    pen(&mut h, at, true, false);
                } else {
                    mouse(&mut h, at, PointerButton::Primary, true, Modifiers::CTRL);
                }
                let end = if route == 2 || route == 4 {
                    at
                } else {
                    at + vec2(60.0, 0.0)
                };
                if use_pen {
                    pen(&mut h, end, true, false);
                    pen(&mut h, end, false, false);
                } else {
                    h.event(Event::PointerMoved(end));
                    h.step();
                    mouse(&mut h, end, PointerButton::Primary, false, Modifiers::CTRL);
                }
            }
            assert!(camera(&h).distance < c.distance, "経路 {route}");
            near(project(camera(&h), point), Vec2::new(at.x, at.y));
            assert!(!h.doc().can_undo());
        }
    }
}

#[test]
fn frame_selected_shortcut_is_local_and_respects_missing_selection_and_modifiers() {
    let mut h = harness(Preferences::default());
    let original = camera(&h);
    h.event(Event::PointerMoved(pos2(900.0, 900.0)));
    key(&mut h, Key::Period, true, Modifiers::NONE);
    assert_eq!(camera(&h), original);
    key(&mut h, Key::Period, false, Modifiers::NONE);
    h.event(Event::PointerMoved(pos2(300.0, 200.0)));
    key(&mut h, Key::Period, true, Modifiers::CTRL);
    assert_eq!(camera(&h), original);
    key(&mut h, Key::Period, false, Modifiers::NONE);
    h.view3d_mut().material = -1;
    key(&mut h, Key::Period, true, Modifiers::NONE);
    assert_eq!(camera(&h), original);
    key(&mut h, Key::Period, false, Modifiers::NONE);
    h.view3d_mut().material = 0;
    key(&mut h, Key::Period, true, Modifiers::NONE);
    assert_ne!(camera(&h).distance, original.distance);
    assert_eq!(camera(&h).yaw, original.yaw);
}

#[test]
fn cancelled_navigation_drops_its_anchor_and_the_next_press_picks_again() {
    let mut h = harness(prefs(OrbitCenter::Surface, ZoomCenter::View));
    let at = pos2(375.0, 215.0);
    mouse(&mut h, at, PointerButton::Secondary, true, Modifiers::NONE);
    h.event(Event::PointerMoved(at + vec2(20.0, 10.0)));
    h.step();
    key(&mut h, Key::Escape, true, Modifiers::NONE);
    assert!(h.view3d().input.navigation.is_none());
    mouse(&mut h, at, PointerButton::Secondary, false, Modifiers::NONE);
    let at = pos2(290.0, 250.0);
    let point = hit(&h, at);
    mouse(&mut h, at, PointerButton::Secondary, true, Modifiers::NONE);
    h.event(Event::PointerMoved(at + vec2(25.0, 0.0)));
    h.step();
    near(project(camera(&h), point), Vec2::new(at.x, at.y));
    h.event(Event::WindowFocused(false));
    h.step();
    assert!(h.view3d().input.navigation.is_none());
}

#[test]
fn navigation_preferences_roundtrip_and_legacy_defaults() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/navigation-settings-test");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{}.conf", std::process::id()));
    for orbit in OrbitCenter::ALL {
        for zoom in [ZoomCenter::View, ZoomCenter::Pointer] {
            let settings = yolu_app::settings::Settings {
                navigation: prefs(orbit, zoom),
                ..Default::default()
            };
            yolu_app::settings::save(&path, &settings).unwrap();
            let (loaded, problems) = yolu_app::settings::load(&path);
            assert_eq!(loaded, settings);
            assert!(problems.is_empty());
            let mut app = AppState::new(32, 32);
            app.load_settings(loaded);
            assert_eq!(app.settings().navigation, settings.navigation);
        }
    }
    std::fs::write(&path, "language=en\n").unwrap();
    assert_eq!(
        yolu_app::settings::load(&path).0.navigation,
        Preferences::default()
    );
    std::fs::write(&path, "view3d_orbit=unknown\nview3d_zoom=unknown\n").unwrap();
    let (loaded, problems) = yolu_app::settings::load(&path);
    assert_eq!(loaded.navigation, Preferences::default());
    assert_eq!(problems.len(), 2);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn navigation_settings_work_in_japanese_and_english() {
    for lang in Lang::ALL {
        let mut app = AppState::new_in(32, 32, lang);
        app.view3d_mut().load_demo();
        let mut ready = false;
        let mut h = common::gpu_thread::builder()
            .with_size(vec2(252.0, 464.0))
            .build_ui_state(
                move |ui, app| {
                    if !ready {
                        YoluApp::setup(ui.ctx());
                        ready = true;
                        ui.ctx().request_repaint();
                        return;
                    }
                    let mut rows = yolu_app::ui::widgets::Rows::new(ui.max_rect(), 8.0);
                    view3d::display::navigation_settings(ui, app, &mut rows, rect());
                },
                app,
            );
        h.run();
        h.get_by_label(lang.pick("視点", "Navigation")).click();
        h.run();
        for center in OrbitCenter::ALL {
            h.get_by_label(center.label(lang)).click();
            h.run();
            assert_eq!(h.prefs().settings.navigation.orbit, center);
        }
        h.get_by_label(lang.pick("ポインタの所へ", "Toward pointer"))
            .click();
        h.run();
        assert_eq!(h.prefs().settings.navigation.zoom, ZoomCenter::Pointer);
        h.get_by_label(lang.pick("選んだ所に合わせる", "Frame Selected"))
            .click();
        h.run();
        assert_eq!(
            h.view3d().camera.target,
            navigation::selected_bounds(h.view3d()).unwrap().center
        );
    }
}

#[test]
fn texture_set_bounds_and_pivot_follow_only_its_shown_faces() {
    use yolu_app::view3d::model::ViewModel;
    let mut h = harness(prefs(OrbitCenter::TextureSet, ZoomCenter::View));
    let first = h.view3d().model.as_ref().unwrap().meshes[0].clone();
    let mut second = first.clone();
    for p in &mut second.positions {
        p.x += 4.0;
    }
    for s in &mut second.submeshes {
        s.material = 1;
    }
    let model = ViewModel::new("Two cubes", vec![first, second], vec![None, None], 7).unwrap();
    h.view3d_mut().set_model(model);
    h.view3d_mut().material = 1;
    let bounds = navigation::selected_bounds(h.view3d()).unwrap();
    assert_eq!(bounds.center, Vec3::new(4.0, 0.0, 0.0));
    assert_eq!(bounds.size(), Vec3::ONE);
    let at = pos2(320.0, 240.0);
    let before = project(camera(&h), bounds.center);
    mouse(&mut h, at, PointerButton::Secondary, true, Modifiers::NONE);
    h.event(Event::PointerMoved(at + vec2(30.0, 20.0)));
    h.step();
    near(project(camera(&h), bounds.center), before);
    mouse(
        &mut h,
        at + vec2(30.0, 20.0),
        PointerButton::Secondary,
        false,
        Modifiers::NONE,
    );
    navigation::frame_selected(h.app_mut(), rect());
    assert_eq!(camera(&h).target, bounds.center);
    for t in h
        .view3d()
        .model
        .as_ref()
        .unwrap()
        .geometry
        .triangles()
        .iter()
        .filter(|t| t.material == 1)
    {
        for p in [t.a, t.b, t.c] {
            let s = project(camera(&h), p);
            assert!(rect().contains(pos2(s.x, s.y)));
        }
    }
    h.view3d_mut().set_hidden(vec![1]);
    assert!(navigation::selected_bounds(h.view3d()).is_none());
    let before = camera(&h);
    navigation::frame_selected(h.app_mut(), rect());
    assert_eq!(camera(&h), before);
    mouse(&mut h, at, PointerButton::Secondary, true, Modifiers::NONE);
    h.event(Event::PointerMoved(at + vec2(30.0, 20.0)));
    h.step();
    near(
        project(camera(&h), before.target),
        project(before, before.target),
    );
}

#[test]
fn settings_panels_and_navigation_do_not_take_the_frame_shortcut() {
    let mut h = harness(Preferences::default());
    h.event(Event::PointerMoved(pos2(300.0, 200.0)));
    let original = camera(&h);
    h.view3d_mut().display.settings_open = true;
    key(&mut h, Key::Period, true, Modifiers::NONE);
    assert_eq!(camera(&h), original);
    key(&mut h, Key::Period, false, Modifiers::NONE);
    h.view3d_mut().display.settings_open = false;
    // ナビゲーション中も視点を置き直さない。
    mouse(
        &mut h,
        pos2(300.0, 200.0),
        PointerButton::Secondary,
        true,
        Modifiers::NONE,
    );
    key(&mut h, Key::Period, true, Modifiers::NONE);
    assert_eq!(camera(&h), original);
}

#[test]
fn view_settings_persist_through_the_real_window_and_restart() {
    use yolu_app::{pen::PenInput, Tab};
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/navigation-window-test")
        .join(std::process::id().to_string());
    std::fs::create_dir_all(&dir).unwrap();
    for lang in Lang::ALL {
        let path = dir.join(lang.pick("ja.conf", "en.conf"));
        yolu_app::settings::save(
            &path,
            &yolu_app::settings::Settings {
                lang,
                ..Default::default()
            },
        )
        .unwrap();
        let build = || {
            let path = path.clone();
            let mut h = common::gpu_thread::builder()
                .with_size(vec2(1100.0, 760.0))
                .renderer(common::shared_gpu::renderer())
                .build_eframe(move |cc| {
                    let mut app = YoluApp::for_context_with_settings(
                        &cc.egui_ctx,
                        Some(path),
                        PenInput::detached(),
                    )
                    .with_render_state(cc.wgpu_render_state.as_ref());
                    app.set_canvas_backend(yolu_app::canvas::gpu::CanvasBackend::Cpu);
                    app
                });
            // 中央は 1 つの組（`common::app` と同じ並び）
            h.state_mut().dock = common::tabbed_center_dock(1100.0);
            h.run();
            h
        };
        let mut h = build();
        h.view3d_mut().load_demo();
        let at = h.state().tab_rects[&Tab::View3d].center();
        h.event(Event::PointerMoved(at));
        h.event(Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        });
        h.step();
        h.event(Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        });
        h.run();
        h.get_by_label(lang.pick(
            "光・環境・トーンマッピング",
            "Light, environment, tone mapping",
        ))
        .click();
        h.run();
        h.get_by_label(lang.pick("視点", "Navigation")).click();
        h.run();
        h.get_by_label(OrbitCenter::Surface.label(lang)).click();
        h.run();
        h.get_by_label(lang.pick("ポインタの所へ", "Toward pointer"))
            .click();
        h.run();
        let chosen = prefs(OrbitCenter::Surface, ZoomCenter::Pointer);
        assert_eq!(yolu_app::settings::load(&path).0.navigation, chosen);
        let corner = crate::settings_corner(&h);
        crate::common::shot::region(
            &mut h,
            lang.pick(
                "view3d_navigation_settings_ja",
                "view3d_navigation_settings_en",
            ),
            corner,
        );
        drop(h);
        let h = build();
        assert_eq!(h.prefs().settings.navigation, chosen);
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn removing_the_model_during_navigation_releases_the_saved_model() {
    let mut h = harness(prefs(OrbitCenter::Surface, ZoomCenter::Pointer));
    mouse(
        &mut h,
        pos2(360.0, 230.0),
        PointerButton::Secondary,
        true,
        Modifiers::NONE,
    );
    assert!(h.view3d().input.navigation.is_some());
    h.view3d_mut().set_hidden(vec![0]);
    h.step();
    assert!(h.view3d().input.navigation.is_none());
    assert!(h.view3d().input.nav.is_none());
}

#[test]
fn review_auto_depth_pan_tracks_the_surface_after_wheel_zoom() {
    for zoom in [ZoomCenter::View, ZoomCenter::Pointer] {
        for use_pen in [false, true] {
            for notches in [-2.0, 2.0] {
                let mut h = harness(prefs(OrbitCenter::Surface, zoom));
                let at = pos2(375.0, 215.0);
                let point = hit(&h, at);
                if use_pen {
                    key(&mut h, Key::Space, true, Modifiers::NONE);
                    pen(&mut h, at, true, false);
                } else {
                    mouse(&mut h, at, PointerButton::Middle, true, Modifiers::NONE);
                }
                let start = at + vec2(20.0, 10.0);
                if use_pen {
                    pen(&mut h, start, true, false);
                } else {
                    h.event(Event::PointerMoved(start));
                    h.step();
                }
                near(project(camera(&h), point), Vec2::new(start.x, start.y));
                let distance = camera(&h).distance;
                h.event(Event::PointerMoved(start));
                h.event(Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Line,
                    delta: vec2(0.0, notches),
                    modifiers: Modifiers::NONE,
                    phase: egui::TouchPhase::Move,
                });
                h.step();
                assert_ne!(camera(&h).distance, distance);
                let before = project(camera(&h), point);
                let delta = vec2(35.0, -18.0);
                if use_pen {
                    pen(&mut h, start + delta, true, false);
                } else {
                    h.event(Event::PointerMoved(start + delta));
                    h.step();
                }
                near(
                    project(camera(&h), point) - before,
                    Vec2::new(delta.x, delta.y),
                );
            }
        }
    }
}

#[test]
fn review_frame_shortcut_preserves_camera_during_stencil_placement() {
    for release_y in [false, true] {
        let mut h = harness(Preferences::default());
        h.stencil_mut()
            .set_image_rgba("Test stencil", 2, 2, &[255; 16])
            .unwrap();
        key(&mut h, Key::Y, true, Modifiers::NONE);
        let at = pos2(350.0, 220.0);
        mouse(&mut h, at, PointerButton::Middle, true, Modifiers::NONE);
        h.event(Event::PointerMoved(at + vec2(30.0, 20.0)));
        h.step();
        assert!(h.stencil().drag.is_some());
        assert!(h.view3d().input.nav.is_none());
        if release_y {
            key(&mut h, Key::Y, false, Modifiers::NONE);
        }
        let before = camera(&h);
        key(&mut h, Key::Period, true, Modifiers::NONE);
        assert_eq!(camera(&h), before, "ステンシルのドラッグ中は視点を変えない");
        navigation::frame_selected(h.app_mut(), rect());
        assert_eq!(camera(&h), before, "ボタンと同じ入口も抑止する");
        key(&mut h, Key::Period, false, Modifiers::NONE);
        mouse(
            &mut h,
            at + vec2(30.0, 20.0),
            PointerButton::Middle,
            false,
            Modifiers::NONE,
        );
        key(&mut h, Key::Y, false, Modifiers::NONE);
        assert!(!h.stencil().handling());
        key(&mut h, Key::Period, true, Modifiers::NONE);
        assert_ne!(camera(&h), before, "操作後は再びフレームできる");
    }
}

/// 注視点の所の、世界の 0.1 の画面の大きさ（透視と正投影を切り替えても変わらないはず）。
fn size_at_target(h: &Harness<'_, Fixture>) -> f32 {
    let c = camera(h);
    c.view(640.0, 480.0).world_radius_to_screen(c.target, 0.1)
}

#[test]
fn alt_right_drag_flicks_into_axis_views_that_turn_orthographic_with_the_setting() {
    for axis_ortho in [true, false] {
        let mut h = harness(Preferences {
            axis_ortho,
            ..Preferences::default()
        });
        h.view3d_mut().camera.yaw = 25.0;
        let size = size_at_target(&h);
        let at = pos2(375.0, 215.0);
        let alt = Modifiers::ALT;
        let d = navigation::FLICK_DISTANCE;
        h.event(Event::ModifiersChanged(alt));
        mouse(&mut h, at, PointerButton::Secondary, true, alt);
        // 閾値の手前では、軸の視点にならず透視のまま
        h.event(Event::PointerMoved(at + vec2(-(d - 1.0), 0.0)));
        h.step();
        assert!(!camera(&h).is_orthographic());
        // 左へ: 隣の軸の視点に切り替え、設定が入なら正投影（大きさは変わらない）
        h.event(Event::PointerMoved(at + vec2(-(d + 1.0), 0.0)));
        h.step();
        let c = camera(&h);
        assert_eq!((c.yaw, c.pitch), (-90.0, 0.0));
        assert_eq!(c.is_orthographic(), axis_ortho);
        assert_eq!(h.view3d().auto_orthographic, axis_ortho);
        assert!((size_at_target(&h) - size).abs() < 1e-3);
        // さらに切り替えても、軸の視点のままで大きさは変わらない
        h.event(Event::PointerMoved(at + vec2(-(d + 1.0), d + 1.0)));
        h.step();
        let c = camera(&h);
        assert_eq!((c.yaw, c.pitch), (-90.0, 90.0));
        assert_eq!(c.is_orthographic(), axis_ortho);
        assert!((size_at_target(&h) - size).abs() < 1e-3);
        mouse(
            &mut h,
            at + vec2(-(d + 1.0), d + 1.0),
            PointerButton::Secondary,
            false,
            alt,
        );
        // 視点は文書を変えない（取り消しに積まない）
        assert!(!h.doc().can_undo());
    }
}

#[test]
fn orbiting_off_an_axis_view_undoes_only_the_automatic_orthographic() {
    let mut h = harness(Preferences::default());
    // 軸の視点（視点のパイと同じ操作）: 正投影へ、自動の印つき
    h.act(yolu_app::state::Action::View3dNav(navigation::NavOp::Axis(
        yolu_core::geometry::AxisView::Front,
    )));
    assert!(camera(&h).is_orthographic());
    assert!(h.view3d().auto_orthographic);
    let at = pos2(320.0, 240.0);
    mouse(&mut h, at, PointerButton::Secondary, true, Modifiers::NONE);
    h.event(Event::PointerMoved(at + vec2(30.0, 0.0)));
    h.step();
    mouse(
        &mut h,
        at + vec2(30.0, 0.0),
        PointerButton::Secondary,
        false,
        Modifiers::NONE,
    );
    assert!(!camera(&h).is_orthographic(), "回して外れたら透視");
    // 手で正投影にしたものは、回しても戻さない
    h.act(yolu_app::state::Action::View3dNav(
        navigation::NavOp::ToggleOrthographic,
    ));
    assert!(camera(&h).is_orthographic());
    assert!(!h.view3d().auto_orthographic);
    mouse(&mut h, at, PointerButton::Secondary, true, Modifiers::NONE);
    h.event(Event::PointerMoved(at + vec2(-40.0, 25.0)));
    h.step();
    mouse(
        &mut h,
        at + vec2(-40.0, 25.0),
        PointerButton::Secondary,
        false,
        Modifiers::NONE,
    );
    assert!(camera(&h).is_orthographic());
    // 手で正投影にした後に軸の視点を選んでも、自動の印は付けない（外れても正投影のまま）
    h.act(yolu_app::state::Action::View3dNav(navigation::NavOp::Axis(
        yolu_core::geometry::AxisView::Top,
    )));
    assert!(camera(&h).is_orthographic() && !h.view3d().auto_orthographic);
    // モデル全体の位置へ戻しても、手で選んだ正投影は保つ（既定の斜めの向き）
    h.view3d_mut().frame_model();
    assert!(camera(&h).is_orthographic());
    assert_eq!(camera(&h).yaw, yolu_core::geometry::DEFAULT_YAW);
    // 自動の正投影なら、全体の位置へ戻すと透視
    h.act(yolu_app::state::Action::View3dNav(
        navigation::NavOp::ToggleOrthographic,
    ));
    h.act(yolu_app::state::Action::View3dNav(navigation::NavOp::Axis(
        yolu_core::geometry::AxisView::Left,
    )));
    assert!(h.view3d().auto_orthographic);
    h.view3d_mut().frame_model();
    assert!(!camera(&h).is_orthographic());
    assert!(!h.doc().can_undo());
}

#[test]
fn orthographic_pan_and_pointer_zoom_follow_the_pointer() {
    for at in [pos2(375.0, 215.0), pos2(30.0, 40.0)] {
        let mut h = harness(prefs(OrbitCenter::Surface, ZoomCenter::Pointer));
        h.view3d_mut().camera.yaw = 25.0;
        h.view3d_mut().camera.set_orthographic(true);
        let c = camera(&h);
        let view = c.view(640.0, 480.0);
        // 面が無ければ、注視点と同じ奥行きの点
        let point = pick(
            &h.view3d().model.as_ref().unwrap().geometry,
            &view,
            Vec2::new(at.x, at.y),
        )
        .map(|p| p.position)
        .unwrap_or(view.point_at_depth(Vec2::new(at.x, at.y), c.distance));
        h.event(Event::PointerMoved(at));
        h.event(Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: vec2(0.0, 1.0),
            modifiers: Modifiers::NONE,
            phase: egui::TouchPhase::Move,
        });
        h.step();
        let after = camera(&h);
        assert!(after.is_orthographic());
        assert!(after.height_at_target() < c.height_at_target(), "寄った");
        assert_eq!(after.distance, c.distance, "カメラの距離は変えない");
        near(project(after, point), Vec2::new(at.x, at.y));
        // 中ボタンのパン: 押した点が指についてくる（奥行きによらない）
        let p = hit(&h, pos2(375.0, 215.0));
        let before = project(camera(&h), p);
        mouse(
            &mut h,
            pos2(375.0, 215.0),
            PointerButton::Middle,
            true,
            Modifiers::NONE,
        );
        h.event(Event::PointerMoved(pos2(405.0, 195.0)));
        h.step();
        mouse(
            &mut h,
            pos2(405.0, 195.0),
            PointerButton::Middle,
            false,
            Modifiers::NONE,
        );
        near(project(camera(&h), p), before + Vec2::new(30.0, -20.0));
    }
}

#[test]
fn the_axis_orthographic_setting_is_on_by_default_and_round_trips() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/navigation-settings-test");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("axis-{}.conf", std::process::id()));
    // 既定は入。入のままなら書かない（前の版の設定のファイルも入として読む）
    assert!(Preferences::default().axis_ortho);
    let on = yolu_app::settings::Settings::default();
    yolu_app::settings::save(&path, &on).unwrap();
    assert!(!std::fs::read_to_string(&path)
        .unwrap()
        .contains("view3d_axis_ortho"));
    // 切ると書き、読み直すと切
    let off = yolu_app::settings::Settings {
        navigation: Preferences {
            axis_ortho: false,
            ..Preferences::default()
        },
        ..Default::default()
    };
    yolu_app::settings::save(&path, &off).unwrap();
    assert!(std::fs::read_to_string(&path)
        .unwrap()
        .contains("view3d_axis_ortho=off\n"));
    let (loaded, problems) = yolu_app::settings::load(&path);
    assert_eq!(loaded, off);
    assert!(problems.is_empty());
    // 知らない値は断って既定（入）
    std::fs::write(&path, "view3d_axis_ortho=maybe\n").unwrap();
    let (loaded, problems) = yolu_app::settings::load(&path);
    assert!(loaded.navigation.axis_ortho);
    assert_eq!(
        problems,
        [yolu_app::settings::Problem::Invalid {
            key: "view3d_axis_ortho",
            value: "maybe".into()
        }]
    );
    std::fs::remove_file(path).unwrap();
}
