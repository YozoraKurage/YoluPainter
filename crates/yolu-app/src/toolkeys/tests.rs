use super::*;
use egui::{Event, Modifiers, RawInput};

use crate::mode::EditorMode;
use crate::state::{OpenPopup, PopupKind};
use crate::ui::menu::PopupState;

/// キーの事象を実際のキーの処理（`shell::handle_shortcuts`）へ 1 フレームずつ流す台。時刻は 1 フレーム 1/60 秒で進む。
struct Rig {
    ctx: egui::Context,
    app: AppState,
    time: f64,
}

const FRAME: f64 = 1.0 / 60.0;

impl Rig {
    fn new() -> Rig {
        Rig {
            ctx: egui::Context::default(),
            app: AppState::new(64, 64),
            time: 0.0,
        }
    }

    fn frame(&mut self, events: Vec<Event>) {
        let input = RawInput {
            events,
            time: Some(self.time),
            ..Default::default()
        };
        self.time += FRAME;
        let app = &mut self.app;
        let mut output = self.ctx.run_ui(input, |ui| {
            crate::keymap::install(app.keys.map());
            crate::shell::handle_shortcuts(ui.ctx(), app);
        });
        output.textures_delta.clear();
    }

    fn key(key: Key, pressed: bool, repeat: bool, modifiers: Modifiers) -> Event {
        Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat,
            modifiers,
        }
    }

    fn down(&mut self, key: Key) {
        self.frame(vec![Self::key(key, true, false, Modifiers::NONE)]);
    }

    fn up(&mut self, key: Key) {
        self.frame(vec![Self::key(key, false, false, Modifiers::NONE)]);
    }

    /// 何も起きないフレームを、`secs` 秒ぶん流す。
    fn idle(&mut self, secs: f64) {
        for _ in 0..(secs / FRAME).round() as usize {
            self.frame(Vec::new());
        }
    }

    fn mode(&mut self, command: &'static str, mode: ToolKeyMode) {
        self.app.apply(Action::ToolKeyMode(command, mode));
    }

    fn tool(&self) -> Tool {
        self.app.tool
    }

    /// ブラシのストロークが始まった状態にする（実際のドラッグは gui_canvas の試験）。
    fn begin_stroke(&mut self) {
        let layer = self.app.selected_layer.unwrap();
        let settings = self.app.stroke_settings(false);
        let stroke = self.app.doc.begin_stroke(layer, &settings).unwrap();
        self.app.stroke = Some(stroke);
        self.app.canvas.stroke = Some(crate::state::StrokeSource::Mouse);
    }

    fn end_stroke(&mut self) {
        let stroke = self.app.stroke.take().unwrap();
        self.app.doc.cancel_stroke(stroke);
        self.app.canvas.stroke = None;
    }
}

#[test]
fn modes_round_trip_through_their_file_names() {
    for mode in ToolKeyMode::ALL {
        assert_eq!(ToolKeyMode::from_key(mode.key()), Some(mode));
    }
    assert_eq!(ToolKeyMode::from_key("toggle"), None);
    assert_eq!(ToolKeyMode::default(), ToolKeyMode::Tap);
}

#[test]
fn every_mode_has_a_name_in_both_languages() {
    for mode in ToolKeyMode::ALL {
        for lang in Lang::ALL {
            assert!(!mode.label(lang).is_empty());
            assert!(!mode.short_label(lang).is_empty());
        }
    }
}

#[test]
fn only_the_tool_commands_have_a_mode() {
    for tool in Tool::ALL {
        assert_eq!(
            commands::selected_tool(commands::tool_command(tool)),
            Some(tool)
        );
    }
    assert_eq!(commands::selected_tool("view.flip"), None);
    assert_eq!(commands::selected_tool("no.such"), None);
    let mut rig = Rig::new();
    rig.mode("view.flip", ToolKeyMode::Hold);
    assert_eq!(rig.app.keys.tool_mode("view.flip"), ToolKeyMode::Tap);
    assert!(rig.app.keys.is_default());
}

#[test]
fn the_default_is_tap_and_the_key_switches_for_good() {
    let mut rig = Rig::new();
    rig.down(Key::E);
    assert_eq!(rig.tool(), Tool::Eraser);
    assert!(!rig.app.temp_tool.is_active());
    rig.idle(1.0);
    rig.up(Key::E);
    assert_eq!(rig.tool(), Tool::Eraser);
}

#[test]
fn hold_mode_returns_to_the_previous_tool_when_the_key_is_released() {
    let mut rig = Rig::new();
    rig.mode("tool.eraser", ToolKeyMode::Hold);
    let brush_slot = rig.app.toolset.set.first_of(Tool::Brush);
    assert!(brush_slot.is_some());
    rig.down(Key::E);
    assert_eq!(rig.tool(), Tool::Eraser);
    assert!(rig.app.temp_tool.is_active());
    assert_eq!(
        rig.app.temp_tool_return_slot(),
        brush_slot,
        "戻る先のツールバーの項目"
    );
    rig.idle(0.1);
    assert_eq!(rig.tool(), Tool::Eraser, "押している間は消しゴム");
    rig.up(Key::E);
    assert_eq!(rig.tool(), Tool::Brush);
    assert!(!rig.app.temp_tool.is_active());
    assert_eq!(rig.app.temp_tool_return_slot(), None);
    assert_eq!(rig.app.toolset.set.active(), brush_slot);
}

#[test]
fn the_press_and_release_in_one_frame_still_return() {
    let mut rig = Rig::new();
    rig.mode("tool.eraser", ToolKeyMode::Hold);
    rig.frame(vec![
        Rig::key(Key::E, true, false, Modifiers::NONE),
        Rig::key(Key::E, false, false, Modifiers::NONE),
    ]);
    assert_eq!(rig.tool(), Tool::Brush);
    assert!(!rig.app.temp_tool.is_active());
}

#[test]
fn repeated_presses_do_nothing_in_hold_modes() {
    let mut rig = Rig::new();
    rig.mode("tool.eraser", ToolKeyMode::Hold);
    rig.down(Key::E);
    // 押している間にツールバーで別のツールを選んでも、キーの繰り返しの押しで消しゴムに取り戻さない
    rig.app.apply(Action::SelectTool(Tool::Fill));
    rig.frame(vec![Rig::key(Key::E, true, true, Modifiers::NONE)]);
    rig.frame(vec![Rig::key(Key::E, true, true, Modifiers::NONE)]);
    assert_eq!(rig.tool(), Tool::Fill);
    rig.up(Key::E);
    assert_eq!(rig.tool(), Tool::Fill, "選んだツールが残る");
}

#[test]
fn tap_or_hold_keeps_the_tool_after_a_short_press_and_returns_after_a_long_one() {
    let mut rig = Rig::new();
    rig.mode("tool.eraser", ToolKeyMode::TapOrHold);
    // 短い押し
    rig.down(Key::E);
    assert_eq!(rig.tool(), Tool::Eraser);
    assert!(rig.app.temp_tool.is_active());
    rig.idle(0.1);
    rig.up(Key::E);
    assert_eq!(rig.tool(), Tool::Eraser, "短く押したら切り替えたまま");
    assert!(!rig.app.temp_tool.is_active());
    // 長押し
    rig.app.apply(Action::SelectTool(Tool::Brush));
    rig.down(Key::E);
    rig.idle(HOLD_AFTER_SECS + 0.1);
    assert_eq!(rig.tool(), Tool::Eraser);
    rig.up(Key::E);
    assert_eq!(rig.tool(), Tool::Brush, "長押しは離すと戻る");
}

#[test]
fn the_hold_time_boundary_is_a_quarter_of_a_second() {
    // 境のすぐ両側（0.24 秒は短い、0.26 秒は長い）
    let mut rig = Rig::new();
    rig.mode("tool.eraser", ToolKeyMode::TapOrHold);
    let t0 = rig.time;
    rig.down(Key::E);
    rig.time = t0 + 0.24;
    rig.up(Key::E);
    assert_eq!(rig.tool(), Tool::Eraser, "0.24 秒は短い押し");
    rig.app.apply(Action::SelectTool(Tool::Brush));
    let t0 = rig.time;
    rig.down(Key::E);
    rig.time = t0 + 0.26;
    rig.up(Key::E);
    assert_eq!(rig.tool(), Tool::Brush, "0.26 秒は長押し");
}

#[test]
fn tap_or_hold_returns_after_a_short_press_if_something_was_drawn_meanwhile() {
    let mut rig = Rig::new();
    rig.mode("tool.eraser", ToolKeyMode::TapOrHold);
    rig.down(Key::E);
    rig.begin_stroke();
    rig.frame(Vec::new());
    rig.end_stroke();
    rig.frame(Vec::new());
    rig.up(Key::E);
    assert_eq!(rig.tool(), Tool::Brush, "短くても、描いたら戻る");
}

#[test]
fn a_release_in_the_middle_of_a_stroke_waits_until_the_stroke_ends() {
    for mode in [ToolKeyMode::Hold, ToolKeyMode::TapOrHold] {
        let mut rig = Rig::new();
        rig.mode("tool.eraser", mode);
        rig.down(Key::E);
        rig.begin_stroke();
        rig.frame(Vec::new());
        rig.up(Key::E);
        rig.idle(0.5);
        assert_eq!(rig.tool(), Tool::Eraser, "{mode:?}: 線の途中では戻さない");
        assert!(rig.app.temp_tool.is_active(), "{mode:?}");
        assert!(rig.app.canvas.stroke.is_some());
        rig.end_stroke();
        rig.frame(Vec::new());
        assert_eq!(rig.tool(), Tool::Brush, "{mode:?}: 描き終えたら戻す");
        assert!(!rig.app.temp_tool.is_active());
    }
}

#[test]
fn choosing_a_tool_by_another_way_while_held_drops_the_return() {
    // ツールバー（ツールの列）
    let mut rig = Rig::new();
    rig.mode("tool.eraser", ToolKeyMode::Hold);
    rig.down(Key::E);
    let fill = rig.app.toolset.set.first_of(Tool::Fill).unwrap();
    rig.app.apply(Action::Tools(ToolsetAction::Select(fill)));
    rig.frame(Vec::new());
    assert!(!rig.app.temp_tool.is_active());
    rig.up(Key::E);
    assert_eq!(rig.tool(), Tool::Fill);
    // ほかのキー（押すと切り替え）
    let mut rig = Rig::new();
    rig.mode("tool.eraser", ToolKeyMode::Hold);
    rig.down(Key::E);
    rig.down(Key::G);
    assert_eq!(rig.tool(), Tool::Fill);
    rig.up(Key::E);
    rig.up(Key::G);
    assert_eq!(rig.tool(), Tool::Fill);
    // 同じ消しゴムをツールバーで選び直しても、そのまま消しゴム
    let mut rig = Rig::new();
    rig.mode("tool.eraser", ToolKeyMode::Hold);
    rig.down(Key::E);
    let eraser = rig.app.toolset.set.first_of(Tool::Eraser).unwrap();
    rig.app.apply(Action::Tools(ToolsetAction::Select(eraser)));
    rig.up(Key::E);
    assert_eq!(rig.tool(), Tool::Eraser);
    // パイ・メニューの項目（操作の実行）
    let mut rig = Rig::new();
    rig.mode("tool.eraser", ToolKeyMode::Hold);
    rig.down(Key::E);
    let action = commands::find("tool.move").unwrap().runnable().unwrap();
    rig.app.apply(action);
    rig.up(Key::E);
    assert_eq!(rig.tool(), Tool::Move);
}

#[test]
fn losing_the_focus_counts_as_a_release() {
    let mut rig = Rig::new();
    rig.mode("tool.eraser", ToolKeyMode::Hold);
    rig.down(Key::E);
    rig.frame(vec![Event::WindowFocused(false)]);
    assert_eq!(rig.tool(), Tool::Brush);
    assert!(!rig.app.temp_tool.is_active());
    // 描いている途中なら、描き終えてから
    rig.down(Key::E);
    rig.begin_stroke();
    rig.frame(Vec::new());
    rig.frame(vec![Event::WindowFocused(true)]);
    rig.down(Key::E);
    rig.frame(vec![Event::WindowFocused(false)]);
    assert_eq!(rig.tool(), Tool::Eraser);
    rig.end_stroke();
    rig.frame(Vec::new());
    assert_eq!(rig.tool(), Tool::Brush);
}

#[test]
fn a_popup_opening_counts_as_a_release() {
    let mut rig = Rig::new();
    rig.mode("tool.eraser", ToolKeyMode::Hold);
    rig.down(Key::E);
    let anchor = egui::Rect::from_min_size(egui::pos2(10.0, 10.0), egui::vec2(10.0, 10.0));
    rig.app.popup = Some(OpenPopup {
        kind: PopupKind::Mode,
        state: PopupState::new(&rig.ctx, anchor),
    });
    rig.frame(Vec::new());
    assert_eq!(rig.tool(), Tool::Brush);
    assert!(!rig.app.temp_tool.is_active());
    // 開いたままキーを離しても、もう何も起きない
    rig.up(Key::E);
    assert_eq!(rig.tool(), Tool::Brush);
}

#[test]
fn pressing_the_key_of_the_tool_already_selected_changes_nothing() {
    let mut rig = Rig::new();
    rig.mode("tool.brush", ToolKeyMode::Hold);
    assert_eq!(rig.tool(), Tool::Brush);
    rig.down(Key::B);
    assert!(!rig.app.temp_tool.is_active());
    rig.up(Key::B);
    assert_eq!(rig.tool(), Tool::Brush);
    // 消しゴムの一時の切り替えのあとで、元のブラシのキーを押しても同じ
    rig.mode("tool.eraser", ToolKeyMode::Hold);
    rig.app.apply(Action::SelectTool(Tool::Eraser));
    rig.down(Key::E);
    assert!(!rig.app.temp_tool.is_active());
    rig.up(Key::E);
    assert_eq!(rig.tool(), Tool::Eraser);
}

#[test]
fn a_modifier_released_first_does_not_end_the_hold() {
    let mut rig = Rig::new();
    rig.mode("tool.gradient", ToolKeyMode::Hold);
    // Shift+G（グラデーション）
    rig.frame(vec![
        Event::ModifiersChanged(Modifiers::SHIFT),
        Rig::key(Key::G, true, false, Modifiers::SHIFT),
    ]);
    assert_eq!(rig.tool(), Tool::Gradient);
    rig.frame(vec![Event::ModifiersChanged(Modifiers::NONE)]);
    rig.idle(0.1);
    assert_eq!(
        rig.tool(),
        Tool::Gradient,
        "Shift を先に離しても、G を押している間"
    );
    // Shift を離したあとの G の繰り返しの押しは、修飾なしの G（バケツ）に当たらない
    for _ in 0..5 {
        rig.frame(vec![Rig::key(Key::G, true, true, Modifiers::NONE)]);
    }
    assert_eq!(
        rig.tool(),
        Tool::Gradient,
        "繰り返しが別の割り当てに当たらない"
    );
    assert!(rig.app.temp_tool.is_active());
    rig.up(Key::G);
    assert_eq!(rig.tool(), Tool::Brush);
}

#[test]
fn hold_keys_stack_and_the_later_key_returns_to_the_earlier_tool() {
    let mut rig = Rig::new();
    rig.mode("tool.eraser", ToolKeyMode::Hold);
    rig.mode("tool.fill", ToolKeyMode::Hold);
    rig.down(Key::E);
    rig.down(Key::G);
    assert_eq!(rig.tool(), Tool::Fill);
    assert_eq!(rig.app.temp_tool.depth(), 2);
    // 後のキーを先に離す: まだ押している先のキーのツール（消しゴム）へ戻り、先のキーの切り替えが続く
    rig.up(Key::G);
    assert_eq!(rig.tool(), Tool::Eraser);
    assert_eq!(rig.app.temp_tool.depth(), 1);
    rig.idle(0.1);
    assert_eq!(rig.tool(), Tool::Eraser, "E をまだ押している間は消しゴム");
    rig.up(Key::E);
    assert_eq!(rig.tool(), Tool::Brush);
    assert!(!rig.app.temp_tool.is_active());
}

#[test]
fn releasing_the_earlier_hold_key_first_keeps_the_later_one_and_it_returns_to_the_first_tool() {
    let mut rig = Rig::new();
    rig.mode("tool.eraser", ToolKeyMode::Hold);
    rig.mode("tool.fill", ToolKeyMode::Hold);
    rig.down(Key::E);
    rig.down(Key::G);
    rig.up(Key::E);
    assert_eq!(rig.tool(), Tool::Fill, "G をまだ押している間はバケツ");
    assert_eq!(rig.app.temp_tool.depth(), 1);
    rig.idle(0.1);
    assert_eq!(rig.tool(), Tool::Fill);
    rig.up(Key::G);
    assert_eq!(rig.tool(), Tool::Brush, "最初のツール（ブラシ）へ");
    assert!(!rig.app.temp_tool.is_active());
}

#[test]
fn both_stacked_keys_released_at_once_return_to_the_first_tool() {
    let mut rig = Rig::new();
    rig.mode("tool.eraser", ToolKeyMode::Hold);
    rig.mode("tool.fill", ToolKeyMode::Hold);
    rig.down(Key::E);
    rig.down(Key::G);
    rig.frame(vec![Event::WindowFocused(false)]);
    assert_eq!(rig.tool(), Tool::Brush);
    assert!(!rig.app.temp_tool.is_active());
}

#[test]
fn a_short_tap_of_the_earlier_key_under_a_held_later_key_keeps_the_earlier_tool() {
    let mut rig = Rig::new();
    rig.mode("tool.eraser", ToolKeyMode::TapOrHold);
    rig.mode("tool.fill", ToolKeyMode::Hold);
    rig.down(Key::E);
    rig.down(Key::G);
    // 先に押した E を、短く（描かずに）離した: 消しゴムに切り替えたことになり、G を離すと消しゴムへ戻る
    rig.up(Key::E);
    assert_eq!(rig.tool(), Tool::Fill);
    rig.idle(0.1);
    rig.up(Key::G);
    assert_eq!(rig.tool(), Tool::Eraser, "短く押した E は切り替えのまま");
    assert!(!rig.app.temp_tool.is_active());
}

#[test]
fn the_stack_keeps_the_rules_of_waiting_for_a_stroke_and_of_other_choices() {
    // 描いている途中に上の段を離したら、描き終えるまで戻さない。そのあと下の段へ戻る
    let mut rig = Rig::new();
    rig.mode("tool.eraser", ToolKeyMode::Hold);
    rig.mode("tool.fill", ToolKeyMode::Hold);
    rig.down(Key::E);
    rig.down(Key::G);
    assert_eq!(rig.tool(), Tool::Fill);
    rig.begin_stroke();
    rig.frame(Vec::new());
    rig.up(Key::G);
    rig.idle(0.2);
    assert_eq!(rig.tool(), Tool::Fill, "描いている途中は戻さない");
    rig.end_stroke();
    rig.frame(Vec::new());
    assert_eq!(rig.tool(), Tool::Eraser, "描き終えたら下の段のツールへ");
    rig.up(Key::E);
    assert_eq!(rig.tool(), Tool::Brush);
    // 別の手段で選んだら、積み重ね全部を捨てる
    let mut rig = Rig::new();
    rig.mode("tool.eraser", ToolKeyMode::Hold);
    rig.mode("tool.fill", ToolKeyMode::Hold);
    rig.down(Key::E);
    rig.down(Key::G);
    let move_tool = commands::find("tool.move").unwrap().runnable().unwrap();
    rig.app.apply(move_tool);
    rig.frame(Vec::new());
    assert!(!rig.app.temp_tool.is_active());
    rig.up(Key::G);
    rig.up(Key::E);
    assert_eq!(rig.tool(), Tool::Move);
}

#[test]
fn pressing_the_same_key_again_while_the_stroke_goes_on_keeps_the_temporary_tool_and_returns_at_the_end(
) {
    for mode in [ToolKeyMode::Hold, ToolKeyMode::TapOrHold] {
        let mut rig = Rig::new();
        rig.mode("tool.eraser", mode);
        rig.down(Key::E);
        rig.begin_stroke();
        rig.frame(Vec::new());
        rig.up(Key::E);
        rig.idle(0.1);
        assert_eq!(
            rig.tool(),
            Tool::Eraser,
            "{mode:?}: 描いている途中は戻さない"
        );
        // 同じ線のうちに押し直す
        rig.down(Key::E);
        assert_eq!(rig.tool(), Tool::Eraser, "{mode:?}");
        assert!(
            rig.app.temp_tool.is_active(),
            "{mode:?}: 続ける（捨てない）"
        );
        rig.idle(0.1);
        rig.end_stroke();
        rig.frame(Vec::new());
        assert_eq!(
            rig.tool(),
            Tool::Eraser,
            "{mode:?}: キーをまた押している間は消しゴム"
        );
        rig.up(Key::E);
        assert_eq!(rig.tool(), Tool::Brush, "{mode:?}: 離したら戻る");
        assert!(!rig.app.temp_tool.is_active(), "{mode:?}");
    }
}

#[test]
fn changing_the_mode_counts_as_a_release_and_returns_the_tool_before_leaving_paint() {
    for target in [EditorMode::Edit, EditorMode::Pose] {
        let mut rig = Rig::new();
        rig.app
            .apply(Action::Pose(crate::view3d::pose::PoseAction::LoadFigure));
        rig.mode("tool.eraser", ToolKeyMode::Hold);
        rig.down(Key::E);
        assert_eq!(rig.tool(), Tool::Eraser);
        assert!(rig.app.set_mode(target), "{target:?}");
        assert_eq!(rig.app.mode, target);
        assert_eq!(
            rig.tool(),
            Tool::Brush,
            "{target:?}: モードを替える前に戻す"
        );
        assert!(!rig.app.temp_tool.is_active());
        // ペイントへ戻っても、一時のツールのまま残らない
        assert!(rig.app.set_mode(EditorMode::Paint));
        assert_eq!(rig.tool(), Tool::Brush, "{target:?}");
        // 離しを受けても、何も起きない
        rig.up(Key::E);
        assert_eq!(rig.tool(), Tool::Brush, "{target:?}");
    }
}

#[test]
fn changing_the_mode_follows_the_tap_or_hold_decision_and_unwinds_a_stack() {
    // 短く押して描いていない TapOrHold は、切り替えのまま
    let mut rig = Rig::new();
    rig.mode("tool.eraser", ToolKeyMode::TapOrHold);
    rig.down(Key::E);
    assert!(rig.app.set_mode(EditorMode::Edit));
    assert_eq!(rig.tool(), Tool::Eraser, "短い押しは切り替えのまま");
    assert!(!rig.app.temp_tool.is_active());
    // 長押しは戻す
    let mut rig = Rig::new();
    rig.mode("tool.eraser", ToolKeyMode::TapOrHold);
    rig.down(Key::E);
    rig.idle(HOLD_AFTER_SECS + 0.1);
    assert!(rig.app.set_mode(EditorMode::Edit));
    assert_eq!(rig.tool(), Tool::Brush);
    // 積み重ねは、上から順に戻して最初のツールへ
    let mut rig = Rig::new();
    rig.mode("tool.eraser", ToolKeyMode::Hold);
    rig.mode("tool.fill", ToolKeyMode::Hold);
    rig.down(Key::E);
    rig.down(Key::G);
    assert!(rig.app.set_mode(EditorMode::Edit));
    assert_eq!(rig.tool(), Tool::Brush);
    assert!(!rig.app.temp_tool.is_active());
}

#[test]
fn a_mode_change_refused_while_stroking_leaves_the_temporary_tool_alone() {
    let mut rig = Rig::new();
    rig.mode("tool.eraser", ToolKeyMode::Hold);
    rig.down(Key::E);
    rig.begin_stroke();
    rig.frame(Vec::new());
    assert!(!rig.app.set_mode(EditorMode::Edit), "描いている間は断る");
    assert_eq!(rig.tool(), Tool::Eraser);
    assert!(rig.app.temp_tool.is_active());
    rig.end_stroke();
}

#[test]
fn a_mode_assigned_behind_set_mode_drops_the_return_without_switching() {
    let mut rig = Rig::new();
    rig.mode("tool.eraser", ToolKeyMode::Hold);
    rig.down(Key::E);
    rig.app.mode = EditorMode::Edit;
    rig.frame(Vec::new());
    assert!(!rig.app.temp_tool.is_active());
    rig.up(Key::E);
    assert_eq!(rig.tool(), Tool::Eraser, "モードの外では戻さない");
    assert_eq!(rig.app.mode, EditorMode::Edit);
}

#[test]
fn a_dialog_or_a_modal_window_opening_counts_as_a_release() {
    let mut rig = Rig::new();
    rig.mode("tool.eraser", ToolKeyMode::Hold);
    rig.down(Key::E);
    rig.app.sel.dialog = Some(crate::selection::AmountDialog {
        kind: crate::selection::ModifyKind::Grow,
        radius: 2,
        edge_lock: false,
    });
    rig.frame(Vec::new());
    assert_eq!(rig.tool(), Tool::Brush, "量を聞くウィンドウが開いた");
    assert!(!rig.app.temp_tool.is_active());
    rig.app.sel.dialog = None;
    rig.up(Key::E);
    // 確認や結果のウィンドウ（モーダル）
    let mut rig = Rig::new();
    rig.mode("tool.eraser", ToolKeyMode::Hold);
    rig.down(Key::E);
    rig.app.brushes.csp.open = true;
    assert!(crate::windows::modal_open(&rig.app));
    rig.frame(Vec::new());
    assert_eq!(rig.tool(), Tool::Brush, "モーダルのウィンドウが開いた");
    assert!(!rig.app.temp_tool.is_active());
}

#[test]
fn the_press_time_follows_the_shared_clock_of_the_main_window() {
    // 押したのは別のウィンドウの時計とは関係なく、メインウィンドウの時刻で測る: 離しをほかのウィンドウのパスが見ても、長さは
    // メインウィンドウの時刻の差になる（ここでは、メインウィンドウのパスが進んでいないので 0 秒で、短い押し）
    let mut rig = Rig::new();
    rig.mode("tool.eraser", ToolKeyMode::TapOrHold);
    rig.down(Key::E);
    let other = egui::ViewportId::from_hash_of("other window");
    let input = RawInput {
        viewport_id: other,
        viewports: [
            (egui::ViewportId::ROOT, Default::default()),
            (other, Default::default()),
        ]
        .into_iter()
        .collect(),
        time: Some(5000.0),
        ..Default::default()
    };
    let app = &mut rig.app;
    let mut output = rig.ctx.run_ui(input, |ui| {
        crate::keymap::install(app.keys.map());
        crate::shell::handle_shortcuts(ui.ctx(), app);
    });
    output.textures_delta.clear();
    assert!(
        !rig.app.temp_tool.is_active(),
        "別のウィンドウのパスは、キーを押していない: 離した"
    );
    assert_eq!(
        rig.tool(),
        Tool::Eraser,
        "長さは 5000 秒ではなく、メインウィンドウの時刻の差（短い）"
    );
}

#[test]
fn the_brush_returns_with_the_tool() {
    let mut rig = Rig::new();
    rig.mode("tool.eraser", ToolKeyMode::Hold);
    let before = ToolPoint::of(&rig.app);
    rig.down(Key::E);
    assert_ne!(ToolPoint::of(&rig.app).brush, before.brush);
    rig.up(Key::E);
    assert_eq!(ToolPoint::of(&rig.app), before, "ブラシも列も戻る");
}

#[test]
fn a_hold_during_a_stroke_does_not_start_a_temporary_switch() {
    let mut rig = Rig::new();
    rig.mode("tool.fill", ToolKeyMode::Hold);
    rig.begin_stroke();
    rig.frame(Vec::new());
    rig.down(Key::G);
    assert!(!rig.app.temp_tool.is_active(), "描いている最中は始めない");
    rig.end_stroke();
}

/// 押している間に、キャンバスのツールの途中の状態（ドラッグ・押し）が始まった・終わったときの、離しの待ち方。`start` で始め、`end` で終える。
fn waits_for(start: impl Fn(&mut AppState), end: impl Fn(&mut AppState), name: &str) {
    for mode in [ToolKeyMode::Hold, ToolKeyMode::TapOrHold] {
        let mut rig = Rig::new();
        rig.mode("tool.eraser", mode);
        rig.down(Key::E);
        start(&mut rig.app);
        rig.frame(Vec::new());
        rig.up(Key::E);
        rig.idle(0.4);
        assert_eq!(rig.tool(), Tool::Eraser, "{name} {mode:?}: 途中は戻さない");
        assert!(rig.app.temp_tool.is_active(), "{name} {mode:?}");
        end(&mut rig.app);
        rig.frame(Vec::new());
        assert_eq!(rig.tool(), Tool::Brush, "{name} {mode:?}: 終わったら戻る");
        assert!(!rig.app.temp_tool.is_active(), "{name} {mode:?}");
    }
}

#[test]
fn every_canvas_tool_drag_and_press_makes_the_release_wait() {
    use crate::state::StrokeSource::Mouse;
    let (a, b) = ((1.0, 2.0), (3.0, 4.0));
    let screen = egui::pos2(10.0, 10.0);
    waits_for(
        |app| {
            app.gradient.drag = Some(crate::gradient::GradientDrag {
                source: Mouse,
                start: a,
                current: b,
                start_screen: screen,
            })
        },
        |app| app.gradient.drag = None,
        "グラデーションのドラッグ",
    );
    waits_for(
        |app| app.text.press = Some((a, Mouse)),
        |app| app.text.press = None,
        "テキストの押し",
    );
    waits_for(
        |app| {
            let layer = app.selected_layer.unwrap();
            app.text.move_press = Some((layer, a, Mouse));
        },
        |app| app.text.move_press = None,
        "移動と変形でのテキストレイヤーの押し",
    );
    for surface in [false, true] {
        waits_for(
            |app| {
                app.path.rect = Some(crate::pathtool::RectDrag {
                    source: Mouse,
                    surface,
                    start: screen,
                    now: screen,
                })
            },
            |app| app.path.rect = None,
            if surface {
                "3D のパスの矩形"
            } else {
                "2D のパスの矩形"
            },
        );
    }
}
