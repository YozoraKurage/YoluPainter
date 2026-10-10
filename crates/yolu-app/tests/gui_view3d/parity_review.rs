//! レビューで見つかった、組み合わせの欠落・色選択の内部表記・設定の書き込みの時機。
use crate::common;
use common::{app, canvas_rect, move_to, press, release, with_render_state_cpu_canvas};
use egui::{accesskit::Role, epaint::Shape, vec2, Event, Key, Modifiers, PointerButton};
use egui_kittest::{kittest::Queryable, Harness};
use yolu_app::{
    lang::Lang,
    pen::PenInput,
    prefs::PrefsAction,
    shortcuts::gestures::{bindings, Operation},
    state::{Action, AppState},
    YoluApp,
};

/// 文書の「既定の割り当て」の表（アプリの表から作る物）のマウスの節の（操作, 組み合わせ）。
fn mouse_rows(lang: Lang) -> Vec<(String, String)> {
    let tables = yolu_app::shortcuts::guide::tables(lang);
    let head = format!("### {}", lang.pick("マウス", "Mouse"));
    let start = tables.find(&head).expect("マウスの節");
    tables[start..]
        .lines()
        .filter(|l| l.starts_with("| ") && !l.starts_with("|---"))
        .skip(1)
        .map(|l| {
            let cells: Vec<&str> = l.trim_matches('|').split(" | ").map(str::trim).collect();
            (cells[0].to_owned(), cells[1].to_owned())
        })
        .collect()
}

#[test]
fn modified_mouse_gestures_are_listed_in_every_view() {
    for lang in Lang::ALL {
        let rows = mouse_rows(lang);
        let left = lang.pick("左ボタン", "Left Button");
        let middle = lang.pick("中ボタン", "Middle Button");
        let right = lang.pick("右ボタン", "Right Button");
        let released = |b: &str| {
            lang.pick(
                format!("{b}を動かさずに離す"),
                format!("{b} Released without Moving"),
            )
        };
        let named = |name: &str, place: &str| {
            let (open, close) = lang.pick(("（", "）"), (" (", ")"));
            format!("{name}{open}{place}{close}")
        };
        let (d2, d3, sel, st) = (
            "2D",
            "3D",
            lang.pick("選択範囲", "Selection"),
            lang.pick("ステンシル", "Stencil"),
        );
        let expected = [
            (named(lang.pick("パン", "Pan"), d2), middle.to_string()),
            (
                named(lang.pick("回転", "Rotate"), d2),
                format!("Alt+{left}"),
            ),
            (
                named(lang.pick("スポイト", "Eyedropper"), d2),
                right.to_string(),
            ),
            (
                named(lang.pick("選択範囲に追加", "Add to Selection"), sel),
                format!("Shift+{left}"),
            ),
            (
                named(
                    lang.pick("選択範囲から引く", "Subtract from Selection"),
                    sel,
                ),
                format!("Ctrl+{left}"),
            ),
            (
                named(
                    lang.pick("選択範囲と重ねる", "Intersect with Selection"),
                    sel,
                ),
                format!("Ctrl+Shift+{left}"),
            ),
            (named(lang.pick("回転", "Orbit"), d3), right.to_string()),
            (
                named(lang.pick("スポイト", "Eyedropper"), d3),
                released(right),
            ),
            (named(lang.pick("パン", "Pan"), d3), middle.to_string()),
            (
                named(lang.pick("スナップ回転", "Snap Orbit"), d3),
                format!("Alt+{left}"),
            ),
            (
                named(lang.pick("クローンの元を決める", "Set Clone Source"), d3),
                format!("Alt+{}", released(left)),
            ),
            (
                named(lang.pick("ステンシルの移動", "Move Stencil"), st),
                format!("Y+Ctrl+{left}"),
            ),
            (
                named(lang.pick("ステンシルの拡縮", "Scale Stencil"), st),
                format!("Y+Alt+{left}"),
            ),
            (
                named(
                    lang.pick(
                        "ステンシルの回転を 15° 刻みに",
                        "Snap Stencil Rotation to 15°",
                    ),
                    st,
                ),
                format!("Y+Shift+{left}"),
            ),
        ];
        let missing: Vec<_> = expected.iter().filter(|e| !rows.contains(e)).collect();
        assert!(missing.is_empty(), "無い組み合わせ: {missing:?}\n{rows:?}");
        // 既定から外した組み合わせは、表に出ない（2D の Shift+中ボタンの回転・Alt+左のスポイト、3D の Shift+右・Alt+Shift+左のパンと Alt+左の自由な回転）
        let gone = [
            (
                named(lang.pick("回転", "Rotate"), d2),
                format!("Shift+{middle}"),
            ),
            (
                named(lang.pick("スポイト", "Eyedropper"), d2),
                format!("Alt+{left}"),
            ),
            (
                named(lang.pick("パン", "Pan"), d3),
                format!("Shift+{right}"),
            ),
            (
                named(lang.pick("パン", "Pan"), d3),
                format!("Alt+Shift+{left}"),
            ),
            (named(lang.pick("回転", "Orbit"), d3), format!("Alt+{left}")),
        ];
        let still: Vec<_> = gone.iter().filter(|g| rows.contains(g)).collect();
        assert!(still.is_empty(), "まだある組み合わせ: {still:?}");
    }
}

fn settings(lang: Lang) -> Harness<'static, AppState> {
    let mut app = AppState::new(32, 32);
    app.lang = lang;
    app.prefs.open = true;
    app.prefs.category = yolu_app::prefs::Category::View3d;
    let mut ready = false;
    let mut h = common::gpu_thread::builder()
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
                yolu_app::panels::color_window::show_in_app(ui.ctx(), app);
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
fn opened_uv_color_window_has_no_internal_notation_and_is_localized() {
    for lang in Lang::ALL {
        let mut h = settings(lang);
        let before = h.state().prefs.settings.uv_wireframe_color;
        assert_color_text(&h, lang);
        h.get_by_role_and_label(
            Role::ColorWell,
            lang.pick(
                "UV ワイヤーフレームの色と不透明度",
                "UV wireframe color and opacity",
            ),
        )
        .click();
        h.run();
        assert!(yolu_app::panels::color_window::is_target(
            &h.ctx,
            yolu_app::uv_wireframe::color::window_target()
        ));
        assert_color_text(&h, lang);
        // 色のウィンドウに不透明度（アルファ）の欄がある
        assert!(
            h.query_by_role_and_label(Role::Slider, "A").is_some(),
            "開いた色のウィンドウに不透明度の欄がありません"
        );
        assert_eq!(before, h.state().prefs.settings.uv_wireframe_color);
    }
}

#[test]
fn the_uv_color_window_changes_the_color_and_the_opacity_separately_in_the_right_language() {
    for lang in Lang::ALL {
        let mut h = settings(lang);
        h.ctx
            .all_styles_mut(|style| style.interaction.tooltip_delay = 0.0);
        h.get_by_role_and_label(
            Role::ColorWell,
            lang.pick(
                "UV ワイヤーフレームの色と不透明度",
                "UV wireframe color and opacity",
            ),
        )
        .click();
        h.run();
        let alpha = h.get_by_role_and_label(Role::Slider, "A").rect();
        h.event(egui::Event::PointerMoved(alpha.center()));
        for _ in 0..8 {
            h.step();
        }
        let tip = lang.pick("不透明度", "Opacity");
        assert!(
            drawn_text(&h).iter().any(|text| text == tip),
            "ツールチップがありません: {tip}"
        );
        assert_color_text(&h, lang);
        let click = |h: &mut Harness<'_, AppState>, at: egui::Pos2| {
            h.event(egui::Event::PointerMoved(at));
            h.event(egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            });
            h.step();
            h.event(egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            });
            h.run();
        };
        // 不透明度を変えても色は変わらない
        let previous = h.state().prefs.settings.uv_wireframe_color;
        click(
            &mut h,
            egui::pos2(alpha.left() + alpha.width() * 0.25, alpha.center().y),
        );
        let after = h.state().prefs.settings.uv_wireframe_color;
        assert_ne!(after[3], previous[3]);
        assert_eq!(after[..3], previous[..3]);
        // 四角で色を変えても不透明度は変わらない
        let window = yolu_app::panels::color_window::rect(&h.ctx).expect("色のウィンドウ");
        let sq =
            yolu_app::panels::color::wheel_square(yolu_app::panels::color_window::wheel_of(window));
        click(&mut h, sq.left_top() + vec2(6.0, 6.0));
        let later = h.state().prefs.settings.uv_wireframe_color;
        assert_ne!(later[..3], after[..3]);
        assert_eq!(later[3], after[3]);
        assert_color_text(&h, lang);
        // 設定は文書の取り消しに積まない
        assert!(!h.state().can_undo());
    }
}

use yolu_app::keyconfig::{Combo, KeyConfig};
use yolu_app::keymap::{Gesture, GESTURES};

/// 押し方（押しながらのキー・ボタン・修飾）。
#[derive(Clone, Copy, Debug)]
struct Press {
    held: Option<Key>,
    button: PointerButton,
    modifiers: Modifiers,
}

impl Press {
    /// 組み合わせの行の範囲の、この組み合わせの押し（押しながらのキーは行のまま）。
    fn of(g: &Gesture, c: Combo) -> Press {
        Press {
            held: g.held.and_then(yolu_app::keymap::hold_key),
            button: c.button,
            modifiers: Modifiers {
                alt: c.alt,
                shift: c.shift,
                ctrl: c.ctrl,
                command: c.ctrl,
                ..Modifiers::NONE
            },
        }
    }

    fn event(&self, at: egui::Pos2, pressed: bool) -> Event {
        Event::PointerButton {
            pos: at,
            button: self.button,
            pressed,
            modifiers: self.modifiers,
        }
    }
}

/// 変えた組み合わせ（既定の表の番号と組み合わせ）を、アプリの割り当てに入れる。
fn configure(keys: &mut KeyConfig, config: &[(u8, Option<Combo>)]) {
    keys.reset_all();
    for &(index, combo) in config {
        keys.set_combo(index, combo);
    }
}

/// 押しながらのキーを押すフレーム（`AppState` だけで回す 3D ビューとステンシル）。
fn hold(ctx: &egui::Context, app: &mut AppState, rect: egui::Rect, key: Option<Key>) {
    let held = key.map(|key| Event::Key {
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
        |ui| yolu_app::stencil::update_keys(ui.ctx(), app),
    );
    output.textures_delta.clear();
}

/// 3D ビューで、押して動かさずに離す（実際の入力処理 `view3d::input::handle`）。押したときに始まった操作（ドラッグの操作と、離しの操作の印）を返す。
fn observe_view3d(config: &[(u8, Option<Combo>)], press: Press) -> Vec<Operation> {
    use yolu_app::view3d::Nav;
    let ctx = egui::Context::default();
    let mut app = AppState::new(32, 32);
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(400.0, 300.0));
    let at = rect.center();
    app.apply(Action::LoadDemoModel);
    // クローンの元は、クローンのブラシのときだけ決まる
    app.tool = yolu_app::state::Tool::Brush;
    app.m2.brush.effect = yolu_app::engine::BrushEffect::Clone {
        offset: Default::default(),
    };
    configure(&mut app.keys, config);
    hold(&ctx, &mut app, rect, press.held);
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(rect),
            events: vec![
                Event::ModifiersChanged(press.modifiers),
                Event::PointerMoved(at),
                press.event(at, true),
            ],
            ..Default::default()
        },
        |ui| yolu_app::view3d::input::handle(ui, &mut app, rect, &[], false),
    );
    output.textures_delta.clear();
    let mut found = Vec::new();
    if let Some((nav, button)) = app.view3d.input.nav {
        assert_eq!(button, press.button, "{press:?}");
        found.push(match nav {
            Nav::Orbit => Operation::Orbit,
            Nav::SnapOrbit => Operation::SnapOrbit,
            Nav::Pan => Operation::Pan,
            Nav::Zoom => Operation::Zoom,
        });
    }
    if let Some(e) = app.view3d.input.eyedrop {
        assert_eq!(e.button, press.button, "{press:?}");
        found.push(Operation::Pick);
    }
    if let Some((_, button)) = app.view3d.input.clone_press {
        assert_eq!(button, press.button, "{press:?}");
        found.push(Operation::CloneSource);
    }
    if !found.is_empty() {
        // ビューの押しは描かない
        assert!(!app.is_stroking(), "{press:?}");
    }
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(rect),
            events: vec![press.event(at, false)],
            ..Default::default()
        },
        |ui| yolu_app::view3d::input::handle(ui, &mut app, rect, &[], false),
    );
    output.textures_delta.clear();
    // 押したボタンを離すと、始めた物は全部終わる（離しの操作は、同じ所で離したので行われる）
    assert!(
        app.view3d.input.nav.is_none()
            && app.view3d.input.eyedrop.is_none()
            && app.view3d.input.clone_press.is_none(),
        "{press:?}"
    );
    if found.contains(&Operation::CloneSource) {
        assert!(app.clone.source.is_some(), "クローンの元が決まる {press:?}");
    }
    found
}

/// ステンシル（押しながらのキーを押して）を押す（実際の入力処理 `stencil::handle_event`）。始まったドラッグの操作を返す。
fn observe_stencil(config: &[(u8, Option<Combo>)], press: Press) -> Vec<Operation> {
    use yolu_app::stencil::DragKind;
    let ctx = egui::Context::default();
    let mut app = AppState::new(32, 32);
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(400.0, 300.0));
    let at = rect.center() + vec2(60.0, 0.0);
    app.stencil
        .set_image_rgba("Sample", 1, 1, &[255; 4])
        .unwrap();
    configure(&mut app.keys, config);
    hold(&ctx, &mut app, rect, press.held);
    let event = press.event(at, true);
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(rect),
            events: vec![
                Event::ModifiersChanged(press.modifiers),
                Event::PointerMoved(at),
                event.clone(),
            ],
            ..Default::default()
        },
        |ui| {
            yolu_app::stencil::update_keys(ui.ctx(), &mut app);
            yolu_app::stencil::handle_event(&mut app, &event, rect, true, &press.modifiers);
        },
    );
    output.textures_delta.clear();
    let Some(drag) = app.stencil.drag else {
        return Vec::new();
    };
    assert_eq!(drag.button, press.button, "{press:?}");
    vec![match drag.kind {
        DragKind::Move => Operation::MoveStencil,
        DragKind::Scale => Operation::ScaleStencil,
        DragKind::Rotate => Operation::RotateStencil,
    }]
}

/// ステンシルを回している間に `modifiers` を押していると、回す角度が 15° 刻みになるか（回すのは、今の割り当ての回す行の組み合わせで始める）。
fn stencil_snaps(config: &[(u8, Option<Combo>)], modifiers: Modifiers) -> bool {
    let ctx = egui::Context::default();
    let mut app = AppState::new(32, 32);
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(400.0, 300.0));
    let start = rect.center() + vec2(60.0, 0.0);
    app.stencil
        .set_image_rgba("Sample", 1, 1, &[255; 4])
        .unwrap();
    configure(&mut app.keys, config);
    let rotate = GESTURES
        .iter()
        .find(|g| g.operation == Operation::RotateStencil)
        .unwrap();
    let press = Press::of(rotate, app.keys.combo(rotate.index).unwrap());
    hold(&ctx, &mut app, rect, press.held);
    let event = press.event(start, true);
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(rect),
            events: vec![
                Event::ModifiersChanged(press.modifiers),
                Event::PointerMoved(start),
                event.clone(),
            ],
            ..Default::default()
        },
        |ui| {
            yolu_app::stencil::update_keys(ui.ctx(), &mut app);
            assert!(yolu_app::stencil::handle_event(
                &mut app,
                &event,
                rect,
                true,
                &press.modifiers
            ));
        },
    );
    output.textures_delta.clear();
    // 中心の回りに 37° 回す（回している間の修飾は、動いたときの修飾）
    let a = 37.0_f32.to_radians();
    let to = rect.center() + vec2(60.0 * a.cos(), 60.0 * a.sin());
    assert!(yolu_app::stencil::handle_event(
        &mut app,
        &Event::PointerMoved(to),
        rect,
        true,
        &modifiers
    ));
    let angle = app.stencil.angle;
    let snapped = (angle / 15.0 - (angle / 15.0).round()).abs() < 1e-3;
    assert!(snapped || (angle - 37.0).abs() < 1.0, "{angle}");
    snapped
}

/// 2D のキャンバスで押す（実際のウィンドウの入力）。選択のツールなら、先に左半分を選び、中ほどの帯をなぞって、選択の組み合わせ方を返す。
/// ほかは、クローンのブラシで押して動かさずに離し、押したときに始まった操作（ドラッグの操作と、離しの操作の印）を返す。
fn observe_canvas(
    h: &mut Harness<'static, YoluApp>,
    config: &[(u8, Option<Combo>)],
    press: Press,
    select: bool,
) -> Vec<Operation> {
    use yolu_app::selection::{SelAction, SelEdit};
    use yolu_app::state::Tool;
    use yolu_core::selection::SelectionCombine;
    {
        let s = &mut h.state_mut().state;
        configure(&mut s.keys, config);
        s.view.angle = 0.0;
        s.clone = Default::default();
        if select {
            s.tool = Tool::SelectRect;
            let (w, hh) = (s.doc.width() as i64, s.doc.height() as i64);
            s.apply(Action::Sel(SelAction::Edit(SelEdit::Rect {
                x0: 0,
                y0: 0,
                x1: w / 2,
                y1: hh,
                mode: SelectionCombine::Replace,
            })));
        } else {
            s.tool = Tool::Brush;
            s.m2.brush.effect = yolu_app::engine::BrushEffect::Clone {
                offset: Default::default(),
            };
        }
    }
    h.run();
    let rect = canvas_rect(h);
    let (w, hh) = {
        let s = &h.state().state;
        (s.doc.width() as f64, s.doc.height() as f64)
    };
    let view = h.state().state.view.view(rect, w as u32, hh as u32);
    let (from, to) = if select {
        (
            view.to_screen(w * 0.25, hh * 0.125),
            view.to_screen(w * 0.75, hh * 0.875),
        )
    } else {
        (rect.center(), rect.center())
    };
    h.event(Event::ModifiersChanged(press.modifiers));
    h.step();
    h.event(Event::PointerMoved(from));
    h.event(press.event(from, true));
    h.step();
    let mut found = Vec::new();
    if !select {
        let s = &h.state().state;
        if s.canvas.panning {
            found.push(Operation::Pan);
        }
        if s.canvas.rotating.is_some() {
            found.push(Operation::Rotate);
        }
        if let Some(e) = s.canvas.eyedrop {
            assert_eq!(e.button, press.button, "{press:?}");
            found.push(Operation::Pick);
        }
        if let Some((_, button)) = s.canvas.clone_press {
            assert_eq!(button, press.button, "{press:?}");
            found.push(Operation::CloneSource);
        }
        if s.canvas.panning || s.canvas.rotating.is_some() {
            assert_eq!(s.canvas.nav_button, Some(press.button), "{press:?}");
        }
        if !found.is_empty() {
            // ビューの押し・スポイト・クローンの元は描かない
            assert!(!s.is_stroking() && s.canvas.stroke.is_none(), "{press:?}");
        }
    } else {
        for i in 1..=4 {
            h.event(Event::PointerMoved(from + (to - from) * (i as f32 / 4.0)));
            h.step();
        }
    }
    h.event(press.event(to, false));
    h.step();
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.run();
    let s = &h.state().state;
    // 押したボタンを離すと、始めた物は全部終わる
    assert!(
        !s.canvas.panning
            && s.canvas.rotating.is_none()
            && s.canvas.eyedrop.is_none()
            && s.canvas.clone_press.is_none()
            && s.canvas.nav_button.is_none()
            && !s.is_stroking(),
        "{press:?}"
    );
    if found.contains(&Operation::CloneSource) {
        // 同じ所で離したので、クローンの元が決まる（表示は回っていない）
        assert!(s.clone.canvas_source_for(s.doc.id()).is_some(), "{press:?}");
        assert_eq!(s.view.angle, 0.0, "{press:?}");
    }
    if select {
        let amount = |x: f64| {
            s.doc
                .selection()
                .map_or(0, |sel| sel.amount((w * x) as u32, (hh * 0.5) as u32))
        };
        // 左半分だけ・重なり・帯だけの 3 か所
        match (amount(1.0 / 16.0), amount(3.0 / 8.0), amount(5.0 / 8.0)) {
            (255, 255, 255) => found.push(Operation::SelectionAdd),
            (255, 0, 0) => found.push(Operation::SelectionSubtract),
            (0, 255, 0) => found.push(Operation::SelectionIntersect),
            (0, 255, 255) => {} // 置き換え（組み合わせ方の行に当たらない、選択のツールの押し）
            other => panic!("{press:?}: 選択範囲が思わない形 {other:?}"),
        }
    }
    found
}

/// この組み合わせの押しで、実際の入力処理が始めた操作（`config` は変えた組み合わせ）。
fn observe(
    h: &mut Harness<'static, YoluApp>,
    g: &Gesture,
    config: &[(u8, Option<Combo>)],
    combo: Combo,
) -> Vec<Operation> {
    let press = Press::of(g, combo);
    match g.scope {
        "canvas" => observe_canvas(h, config, press, false),
        "selection" => observe_canvas(h, config, press, true),
        "view3d" => observe_view3d(config, press),
        // 回す間の修飾の行は、回している間に効くか
        "stencil" if !g.starts => {
            if stencil_snaps(config, press.modifiers) {
                vec![Operation::SnapStencilRotation]
            } else {
                Vec::new()
            }
        }
        "stencil" => observe_stencil(config, press),
        other => panic!("一覧にない範囲: {other}"),
    }
}

/// 既定の表の全部の行が、その組み合わせの押しで、実際の入力処理で効く。
#[test]
fn every_listed_mouse_gesture_matches_the_real_input_handler() {
    let all = bindings();
    for scope in ["canvas", "selection", "view3d", "stencil"] {
        assert!(
            all.iter().any(|b| b.scope == scope),
            "{scope} の組み合わせがありません"
        );
    }
    let mut h = app(1280.0, 800.0, 256);
    for g in GESTURES.iter() {
        let found = observe(&mut h, g, &[], Combo::of(g));
        assert!(found.contains(&g.operation), "{g:?}: {found:?}");
    }
}

/// 表のどの行も、ボタンか修飾を替えると（設定の画面がぶつかりとして断らない組み合わせなら全部）、実際の入力の道で新しい組み合わせが効き、
/// 古い組み合わせは効かない。古い組み合わせが新しい組み合わせに修飾を足しただけで、書いていない修飾を足した押しも当たる行（視点・選択・
/// ステンシルの行）なら、古い押しは新しい組み合わせの押しに当たるため、効かないことは見ない。回す間の修飾の行は修飾だけを替える。
#[test]
fn changing_the_button_or_the_modifiers_of_any_mouse_row_moves_its_operation_to_the_new_combination(
) {
    use egui::PointerButton::{Extra1, Extra2, Middle, Primary, Secondary};
    let mut h = app(1280.0, 800.0, 256);
    for g in GESTURES.iter() {
        let old = Combo::of(g);
        let mut candidates: Vec<(bool, Combo)> = [Primary, Secondary, Middle, Extra1, Extra2]
            .into_iter()
            .filter(|b| *b != old.button)
            .map(|button| (true, Combo { button, ..old }))
            .collect();
        candidates.extend((0..8u8).map(|bits| {
            (
                false,
                Combo {
                    alt: bits & 1 != 0,
                    shift: bits & 2 != 0,
                    ctrl: bits & 4 != 0,
                    ..old
                },
            )
        }));
        let mut accepted = [0usize; 2];
        for (by_button, combo) in candidates {
            // 回す間の修飾の行は修飾だけ（ボタンは回す行のボタン。修飾の無い物は入れられない）
            let Some(combo) = combo.fit(g) else {
                continue;
            };
            if combo == old {
                continue;
            }
            let mut keys = KeyConfig::default();
            keys.set_combo(g.index, Some(combo));
            if keys.combo_conflict(g.index).is_some() || keys.has_conflicts() {
                continue;
            }
            accepted[usize::from(!by_button)] += 1;
            let config = [(g.index, Some(combo))];
            let now = observe(&mut h, g, &config, combo);
            assert!(
                now.contains(&g.operation),
                "{g:?} を {combo:?} にした: 新しい組み合わせで {now:?}"
            );
            let exact = g.click || (g.scope == "canvas" && g.operation == Operation::Pick);
            let looser = !exact
                && combo.button == old.button
                && (!combo.alt || old.alt)
                && (!combo.shift || old.shift)
                && (!combo.ctrl || old.ctrl);
            if !looser {
                let before = observe(&mut h, g, &config, old);
                assert!(
                    !before.contains(&g.operation),
                    "{g:?} を {combo:?} にした: 古い組み合わせで {before:?}"
                );
            }
        }
        if g.starts || g.click {
            assert!(accepted[0] > 0, "{g:?}: ボタンを替えられる組み合わせが無い");
        }
        assert!(accepted[1] > 0, "{g:?}: 修飾を替えられる組み合わせが無い");
    }
}

#[test]
fn only_the_eyedropper_tool_picks_on_a_left_press_and_every_other_tool_picks_with_the_right_button()
{
    use yolu_app::state::Tool;
    let mut app = AppState::new(16, 16);
    let picking: Vec<Tool> = Tool::ALL
        .into_iter()
        .filter(|tool| {
            app.tool = *tool;
            yolu_app::eyedrop::picks(&app)
        })
        .collect();
    // 一覧の「スポイト」の左ボタンの行は無い（描くツールの Alt + 左は、表示を回す組み合わせになった）。右ボタンの行は、どのツールからでも
    assert_eq!(picking, [Tool::Eyedropper]);
    assert!(bindings()
        .iter()
        .all(|b| !(b.operation == Operation::Pick && b.button == PointerButton::Primary)));
    assert!(bindings().iter().any(|b| b.operation == Operation::Pick
        && b.button == PointerButton::Secondary
        && b.scope == "canvas"));
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
            key: Key::Y,
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
                    &mut app,
                    &press,
                    rect,
                    true,
                    &Modifiers::NONE
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
    let mut h = common::gpu_thread::builder()
        .with_size(vec2(1280.0, 800.0))
        .with_pixels_per_point(1.0)
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(120)
        .renderer(common::shared_gpu::renderer())
        .build_eframe(move |cc| {
            with_render_state_cpu_canvas(
                YoluApp::for_context_with_settings(&cc.egui_ctx, Some(path), PenInput::detached()),
                cc.wgpu_render_state.as_ref(),
            )
        });
    // 中央は 1 つの組（`common::app` と同じ並び）
    h.state_mut().dock = common::tabbed_center_dock(1280.0);
    h.run();
    h
}

/// UV ワイヤーフレームの色は、色のウィンドウでドラッグしている間は設定のファイルへ書かず、離したときに 1 回だけ書く（退避の数のスライダーと同じ）。
#[test]
fn dragging_in_the_uv_color_window_writes_the_settings_once_on_release() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/parity-review-tests")
        .join(std::process::id().to_string());
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("settings.conf");
    std::fs::write(&path, "language=en\n").unwrap();
    let mut h = app_with_settings(&path);
    h.state_mut().state.apply(Action::Prefs(PrefsAction::OpenAt(
        yolu_app::prefs::Category::View3d,
    )));
    h.run();
    h.get_by_role_and_label(Role::ColorWell, "UV wireframe color and opacity")
        .click();
    h.run();
    let window = yolu_app::panels::color_window::rect(&h.ctx).expect("色のウィンドウ");
    let sq =
        yolu_app::panels::color::wheel_square(yolu_app::panels::color_window::wheel_of(window));
    let color = |h: &Harness<'_, YoluApp>| h.state().state.prefs.settings.uv_wireframe_color;
    // 書いたかどうかは、ファイルを見張り用の中身に替えておき、書き換えられたかで見る
    let watch = || std::fs::write(&path, "watch\n").unwrap();
    let untouched = || {
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "watch\n",
            "ドラッグの間は書いてはいけない"
        )
    };
    let default_color = color(&h);
    watch();
    press(&h, sq.left_top() + vec2(10.0, 10.0), PointerButton::Primary);
    h.step();
    h.step();
    assert_ne!(color(&h), default_color, "値は動く");
    assert!(h.state().state.prefs.dragging);
    untouched();
    move_to(&h, sq.center());
    h.step();
    h.step();
    assert!(h.state().state.prefs.dragging);
    untouched();
    // 離すと、そのときの値を 1 回だけ書く（続くフレームでは書き直さない）
    release(&h, sq.center(), PointerButton::Primary);
    h.step();
    h.run();
    assert!(!h.state().state.prefs.dragging);
    let c = color(&h);
    assert_eq!(c[3], default_color[3], "不透明度はそのまま");
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        format!(
            "language=en\nuv_wireframe_color={},{},{},{}\n",
            c[0], c[1], c[2], c[3]
        )
    );
    watch();
    h.step();
    h.step();
    untouched();
    let _ = std::fs::remove_dir_all(&dir);
}
