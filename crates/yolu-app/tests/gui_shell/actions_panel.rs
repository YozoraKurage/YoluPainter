//! アクションのパネル（ドックのタブ「アクション」）: 記録の開始と止め・記録しなかった印・選んで再生・ダブルクリックで名前の変更・
//! ドラッグで並べ替え・右クリックのメニューと、日英の見た目。
use crate::common;
use crate::common::tmp::test_dir;
use egui::{vec2, Event, Modifiers, PointerButton, Pos2};
use egui_kittest::{kittest::Queryable, Harness};
use serde_json::json;
use yolu_app::automation::AutomationOp;
use yolu_app::lang::Lang;
use yolu_app::panels::actions;
use yolu_app::state::{Action, AppState};
use yolu_app::YoluApp;
use yolu_core::Rgba8;

fn harness(app: AppState, size: egui::Vec2) -> Harness<'static, AppState> {
    let mut ready = false;
    let mut h = common::gpu_thread::builder()
        .with_size(size)
        .with_pixels_per_point(1.0)
        .with_step_dt(1.0 / 60.0) // ダブルクリックの間に収まる間隔
        .renderer(common::shared_gpu::renderer())
        .build_ui_state(
            move |ui, app| {
                if !ready {
                    YoluApp::setup(ui.ctx());
                    ready = true;
                    ui.ctx().request_repaint();
                    return;
                }
                actions::show(ui, app);
            },
            app,
        );
    h.run();
    h
}

fn key(h: &Harness<'_, AppState>, key: egui::Key, modifiers: Modifiers) {
    for pressed in [true, false] {
        h.event(Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers,
        });
    }
}

/// アクションを 3 つ置いたアプリ（設定のフォルダは試験の一時のフォルダ）。
fn sample(lang: Lang) -> AppState {
    let mut app = AppState::new_in(64, 64, lang);
    yolu_app::automation::attach(&mut app, test_dir("actions-panel"));
    let names = match lang {
        Lang::Ja => ["汚しの下地", "縁の摩耗", "マスクを反転"],
        Lang::En => ["Grime Base", "Edge Wear", "Invert Mask"],
    };
    for name in names {
        let command = yolu_ops::parse_command(
            &json!({"command": "layer.add", "args": {"kind": "paint", "name": name}}),
        )
        .unwrap();
        app.automation.store.add(name, vec![command]).unwrap();
    }
    app.automation.selected = Some(1);
    app
}

#[test]
fn actions_panel_snapshot() {
    let mut snapshots = egui_kittest::SnapshotResults::new();
    for lang in Lang::ALL {
        let mut h = harness(sample(lang), vec2(300.0, 160.0));
        h.run();
        h.snapshot(lang.pick("actions_panel", "actions_panel_english"));
        snapshots.extend_harness(&mut h);
        // 記録中（記録しなかった操作があった印つき）
        h.state_mut()
            .apply(Action::Automation(AutomationOp::StartRecording));
        let id = h.state().selected_layer.unwrap();
        h.state_mut()
            .doc
            .set_pixel(id, 1, 1, Rgba8::new(255, 0, 0, 255))
            .unwrap();
        h.run();
        assert!(h.state().automation.skipped());
        h.snapshot(lang.pick("actions_panel_recording", "actions_panel_recording_english"));
        snapshots.extend_harness(&mut h);
        // 右クリックのメニュー
        let mut h = harness(sample(lang), vec2(300.0, 240.0));
        let name = lang.pick("縁の摩耗", "Edge Wear");
        h.get_by_label(name).click_secondary();
        h.run();
        h.snapshot(lang.pick("actions_panel_menu", "actions_panel_menu_english"));
        snapshots.extend_harness(&mut h);
    }
}

#[test]
fn record_play_rename_and_reorder_from_the_panel() {
    let mut h = harness(sample(Lang::Ja), vec2(300.0, 200.0));
    // 選んで再生
    h.get_by_label("マスクを反転").click();
    h.run();
    assert_eq!(h.state().automation.selected, Some(2));
    h.get_by_label("再生").click();
    h.run();
    assert!(h
        .state()
        .doc
        .layers()
        .iter()
        .any(|l| l.name() == "マスクを反転"));
    // 記録の開始と止め（何も記録しなければ保存しない）
    h.get_by_label("記録").click();
    h.run();
    assert!(h.state().automation.is_recording());
    h.get_by_label("記録を止める").click();
    h.run();
    assert!(!h.state().automation.is_recording());
    assert_eq!(h.state().automation.store.len(), 3);
    // ダブルクリックで名前の変更
    let at = h.get_by_label("縁の摩耗").rect().center();
    for _ in 0..2 {
        h.event(Event::PointerMoved(at));
        for pressed in [true, false] {
            h.event(Event::PointerButton {
                pos: at,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            });
        }
        h.step();
    }
    h.run();
    assert_eq!(h.state().automation.renaming, Some(1));
    key(&h, egui::Key::A, Modifiers::COMMAND);
    h.event(Event::Text("縁の摩耗2".into()));
    key(&h, egui::Key::Enter, Modifiers::NONE);
    h.run();
    assert_eq!(h.state().automation.store.get(1).unwrap().name, "縁の摩耗2");
    // ドラッグで並べ替え（先頭を最後へ）
    let a = h.get_by_label("汚しの下地").rect().center();
    let c = h.get_by_label("マスクを反転").rect();
    let to = Pos2::new(a.x, c.bottom() - 2.0);
    h.input_mut().events.push(Event::PointerMoved(a));
    h.input_mut().events.push(Event::PointerButton {
        pos: a,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    h.run();
    for step in 1..=6 {
        let p = a + (to - a) * (step as f32 / 6.0);
        h.input_mut().events.push(Event::PointerMoved(p));
        h.run();
    }
    h.input_mut().events.push(Event::PointerButton {
        pos: to,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    h.run();
    let names: Vec<String> = h
        .state()
        .automation
        .store
        .items()
        .iter()
        .map(|s| s.name.clone())
        .collect();
    assert_eq!(names, ["縁の摩耗2", "マスクを反転", "汚しの下地"]);
    // 削除
    h.get_by_label("汚しの下地").click();
    h.run();
    h.get_by_label("削除").click();
    h.run();
    assert_eq!(h.state().automation.store.len(), 2);
}
