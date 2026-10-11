//! モード（ペイント・編集・ポーズ）。描くのはペイントだけで、編集とポーズは置いた物（ポーズはボーン）を選んで動かす。
//!
//! - 状態は `AppState::mode` の 1 つ。替えるのは `AppState::set_mode` だけ（描いている間は断る。ギズモ・点のドラッグの途中なら、そこまでを
//!   確定してから替える。ポーズはボーンのあるモデルが要る）。
//! - 入り方: オプションバーの左端のドロップダウン（`dropdown`）、Ctrl+Tab のモードのパイ（`pie`）、表示のメニュー（`view_menu_entry`）。
//!   骨の木でボーンを選ぶとポーズへ、描くツールを選ぶ（ツールを替える口 `AppState::switch_to`）とペイントへ。
//! - 編集・ポーズの画面: 左のツールの帯は「選択・移動・回転・拡縮」の 4 つ（`edit_strip`）、ブラシと色の欄は暗くして押せない（`dims_tab`）。
//!   2D のキャンバスは見るだけ（視点の操作とスポイトは効く）、3D は描き始めない。
//! - キーの段: 表の行の効く範囲（`keymap::Scope`）がモードを見る。ペイントの行（ツールのキーなど）は、ペイントのモードの間だけ効く。

use egui::{pos2, vec2, Rect, Sense, Ui, WidgetInfo, WidgetType};

use crate::lang::Lang;
use crate::notice::Source;
use crate::state::{Action, AppState, OpenPopup, PopupKind};
use crate::ui::menu::{Entry, PopupState};
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, Align};

/// モード。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum EditorMode {
    /// 描く（ツールの帯のツール・ブラシ・色）。
    #[default]
    Paint,
    /// 置いた物（投影の箱・デカール・形のグラデーション・3D パス・点）を選んで動かす。描かない。
    Edit,
    /// ボーンを選んで動かす。描かない。
    Pose,
}

impl EditorMode {
    pub const ALL: [EditorMode; 3] = [Self::Paint, Self::Edit, Self::Pose];

    pub fn name(self, lang: Lang) -> &'static str {
        match self {
            Self::Paint => lang.pick("ペイント", "Paint"),
            Self::Edit => lang.pick("編集", "Edit"),
            Self::Pose => lang.pick("ポーズ", "Pose"),
        }
    }

    /// アイコン（ドロップダウン・パイ）。
    pub fn icon(self) -> &'static str {
        match self {
            Self::Paint => "paint_brush",
            Self::Edit => "arrow_move",
            Self::Pose => "accessibility",
        }
    }

    /// このモードにする操作の ID（`commands`）。
    pub fn command(self) -> &'static str {
        match self {
            Self::Paint => "mode.paint",
            Self::Edit => "mode.edit",
            Self::Pose => "mode.pose",
        }
    }

    /// 描くモードか（ツール・ブラシ・色が効く）。
    pub fn paints(self) -> bool {
        self == Self::Paint
    }
}

/// 編集・ポーズのモードのツールの帯の 4 つ（キーは無くても使える）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum EditTool {
    #[default]
    Select,
    Move,
    Rotate,
    Scale,
}

impl EditTool {
    pub const ALL: [EditTool; 4] = [Self::Select, Self::Move, Self::Rotate, Self::Scale];

    pub fn name(self, lang: Lang) -> &'static str {
        match self {
            Self::Select => lang.pick("選択", "Select"),
            Self::Move => lang.pick("移動", "Move"),
            Self::Rotate => lang.pick("回転", "Rotate"),
            Self::Scale => lang.pick("拡縮", "Scale"),
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Self::Select => "tools/select-rectangle",
            Self::Move => "arrow_move",
            Self::Rotate => "3d_rotation",
            Self::Scale => "arrow_maximize",
        }
    }

    /// 移動・回転・拡縮のツールが始める G/R/S の種類（選択は None）。
    pub fn transform(self) -> Option<crate::objects::Kind> {
        use crate::objects::Kind;
        match self {
            Self::Select => None,
            Self::Move => Some(Kind::Grab),
            Self::Rotate => Some(Kind::Rotate),
            Self::Scale => Some(Kind::Scale),
        }
    }

    /// 帯で光らせるツール: G/R/S の途中はその種類のツール（キーで始めても）、ほかは選んでいるツール。
    pub fn shown(app: &AppState) -> EditTool {
        match app.objects.transform.as_ref().map(|t| t.kind) {
            Some(crate::objects::Kind::Grab) => Self::Move,
            Some(crate::objects::Kind::Rotate) => Self::Rotate,
            Some(crate::objects::Kind::Scale) => Self::Scale,
            None => app.edit_tool,
        }
    }

    /// 試験・読み上げの部品の名前に添える札。
    fn key(self) -> &'static str {
        match self {
            Self::Select => "select",
            Self::Move => "move",
            Self::Rotate => "rotate",
            Self::Scale => "scale",
        }
    }
}

/// モードの操作（ドロップダウン・パイ・メニュー・ツールの帯から）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModeAction {
    /// このモードにする。
    Set(EditorMode),
    /// 編集・ポーズのツールの帯のツールを選ぶ。
    Tool(EditTool),
}

/// ポーズにできない理由（ボーンのあるモデルが無い）。
pub fn no_rig(lang: Lang) -> &'static str {
    lang.pick("ボーンのあるモデルがありません", "No model with bones")
}

/// ブラシ・色の欄を編集・ポーズのモードで押せない理由。
pub fn paint_only(lang: Lang) -> &'static str {
    lang.pick("ペイントのモードで使います", "Used in Paint mode")
}

impl AppState {
    /// モードを替える（替わったか、もうそのモードなら true）。描いている間は断る（ポーズの断りと同じ文）。ポーズはボーンのあるモデルが要る。
    /// 替える前に、ポーズのギズモ・形のギズモ・点のドラッグの途中なら、そこまでを確定する。ペイントを出るときは、ツールの途中の形
    /// （選択の形・移動と変形・グラデーション・図形・打っている文字）を、ツールを替えたときと同じく終える。
    pub fn set_mode(&mut self, mode: EditorMode) -> bool {
        if self.mode == mode {
            return true;
        }
        // 3D ビューで離した後の残りを塗っている途中（確定待ち）は、先に確定する（断らない）
        crate::view3d::input::settle(self);
        if self.is_stroking() {
            self.refuse(
                Source::Edit,
                crate::lang::refusals::during_stroke(self.lang),
            );
            return false;
        }
        if mode == EditorMode::Pose && self.view3d.pose.session.is_none() {
            let text = self.lang.with_reason(
                self.lang
                    .pick("ポーズのモードにできません", "Cannot enter Pose mode"),
                no_rig(self.lang),
            );
            self.refuse(Source::Pose, text);
            return false;
        }
        // キーを押している間だけのツールは、離したものとして、押す前のツールへ戻してから替える（ペイントへ戻ったとき、一時のツールのまま残さない）
        if self.mode == EditorMode::Paint {
            crate::toolkeys::leave_paint(self);
        }
        // 途中の操作は確定してから替える（G/R/S は決める。面に乗らないパスはやめる）
        crate::objects::transform::finish(self, true);
        crate::view3d::gizmo::release(self, true);
        crate::fillfx::gizmo::release(self, true);
        crate::fillfx::points::release(self, true);
        if self.mode == EditorMode::Paint {
            self.sel_tool_changed();
            self.transform_cancel_drag();
            self.gradient_cancel_drag();
            self.drafting_cancel();
            self.text_commit();
            self.text.press = None;
        }
        self.mode = mode;
        true
    }

    /// 右ボタンがポリゴン塗りつぶしのアイランドのメニューになるか（ペイントのモードでポリゴン塗りつぶしのツールのときだけ。ほかはスポイト）。
    pub fn right_opens_island_menu(&self) -> bool {
        self.mode.paints() && self.tool == crate::state::Tool::PolygonFill
    }

    /// モードの操作を当てる（`Action::Mode`）。
    pub fn mode_apply(&mut self, action: ModeAction) {
        match action {
            ModeAction::Set(mode) => {
                self.set_mode(mode);
            }
            ModeAction::Tool(tool) => self.edit_tool = tool,
        }
    }
}

/// ポーズのセッションが無くなったら（ほかのモデルに替わった）、ポーズのモードを出る（フレームの初め）。
pub fn leave_pose_without_rig(app: &mut AppState) {
    if app.mode == EditorMode::Pose && app.view3d.pose.session.is_none() {
        app.mode = EditorMode::Paint;
    }
}

/// モードを選ぶ項目（ドロップダウン・表示のメニュー。アイコンつきの 3 行。今のモードに印）。ポーズはボーンのあるモデルが無ければ押せず、理由を出す。
pub fn entries(app: &AppState) -> Vec<Entry<Action>> {
    let lang = app.lang;
    let free = !app.is_stroking();
    EditorMode::ALL
        .into_iter()
        .map(|mode| {
            let entry = Entry::item(mode.name(lang), Action::Mode(ModeAction::Set(mode)))
                .icon(mode.icon(), app.mode == mode);
            match mode {
                EditorMode::Pose if app.view3d.pose.session.is_none() => {
                    entry.enabled(false).tooltip(no_rig(lang))
                }
                _ if !free && app.mode != mode => entry
                    .enabled(false)
                    .tooltip(crate::lang::refusals::during_stroke(lang)),
                _ => entry,
            }
        })
        .collect()
}

/// 表示のメニューの「モード」（入れ子の 3 行）。
pub fn view_menu_entry(app: &AppState) -> Entry<Action> {
    Entry::submenu(app.lang.pick("モード", "Mode"), entries(app))
}

/// オプションバーの左端のドロップダウンの幅。
pub const DROPDOWN_WIDTH: f32 = 132.0;

/// オプションバーの左端のドロップダウン（今のモードのアイコンと名前と ▼。押すとアイコンつきの 3 行が開く）。右の端の x を返す。
pub fn dropdown(ui: &mut Ui, app: &mut AppState, bar: Rect, x: f32) -> f32 {
    let lang = app.lang;
    let r = Rect::from_min_size(
        pos2(x, bar.top() + 5.0),
        vec2(DROPDOWN_WIDTH, bar.height() - 10.0),
    );
    let open = matches!(app.popup.as_ref().map(|p| p.kind), Some(PopupKind::Mode));
    let id = ui.make_persistent_id("options.mode");
    let response = ui.interact(r, id, Sense::click());
    let live = !w::stroke_held(ui.ctx());
    let hover = live && response.hovered();
    let p = ui.painter();
    let fill = if open {
        t::ACCENT_DIM
    } else if live && response.is_pointer_button_down_on() {
        t::CONTROL_ACTIVE
    } else if hover {
        t::CONTROL_HOVER
    } else {
        t::PANEL_HEADER
    };
    w::rounded(p, r, fill, 4.0);
    let color = if open || hover {
        egui::Color32::WHITE
    } else {
        t::TEXT
    };
    let mode = app.mode;
    w::icon(
        p,
        Rect::from_min_size(pos2(r.left() + 6.0, r.top()), vec2(20.0, r.height())),
        mode.icon(),
        color,
        18.0,
    );
    w::text(
        p,
        Rect::from_min_max(
            pos2(r.left() + 32.0, r.top()),
            pos2(r.right() - 22.0, r.bottom()),
        ),
        mode.name(lang),
        t::LABEL.with_color(color),
        Align::Left,
    );
    w::icon(
        p,
        Rect::from_min_size(pos2(r.right() - 22.0, r.top()), vec2(18.0, r.height())),
        "expand_more",
        if open { color } else { t::TEXT_DIM },
        16.0,
    );
    let name = format!("{}: {}", lang.pick("モード", "Mode"), mode.name(lang));
    response.widget_info(|| WidgetInfo::labeled(WidgetType::ComboBox, true, &name));
    let tip = match crate::shortcuts::menu_key("mode.pie") {
        Some(key) => lang.pick(format!("モード（{key}）"), format!("Mode ({key})")),
        None => lang.pick("モード", "Mode").to_owned(),
    };
    let clicked = response.on_hover_text(tip).clicked();
    if clicked {
        if open {
            app.popup = None;
        } else {
            let anchor =
                Rect::from_min_size(pos2(r.left(), r.bottom() + 2.0), vec2(r.width(), 0.0));
            let mut state = PopupState::new(ui.ctx(), anchor).with_min_width(r.width());
            state.selected = EditorMode::ALL.iter().position(|m| *m == mode);
            app.popup = Some(OpenPopup {
                kind: PopupKind::Mode,
                state,
            });
        }
    }
    r.right()
}

/// 編集・ポーズのモードのツールの帯（選択・移動・回転・拡縮）。`bottom` は使える下の端。
pub fn edit_strip(ui: &mut Ui, app: &mut AppState, r: Rect, bottom: f32) {
    let lang = app.lang;
    let mut y = r.top() + 6.0;
    for tool in EditTool::ALL {
        let at = Rect::from_min_size(pos2(r.left() + 5.0, y), vec2(r.width() - 10.0, 32.0));
        if at.bottom() > bottom {
            break;
        }
        if w::icon_button(
            ui,
            at,
            ("mode.tool", tool.key()),
            tool.icon(),
            tool.name(lang),
            EditTool::shown(app) == tool,
            true,
            22.0,
        )
        .clicked()
        {
            app.apply(Action::Mode(ModeAction::Tool(tool)));
        }
        y += 34.0;
    }
}

/// オプションバーの、G/R/S のスナップ（常にする・移動・回転・拡縮の刻み。Ctrl を押している間は、常にするの逆）。
pub fn snap_options(ui: &mut Ui, app: &mut AppState, bar: Rect, x: f32) {
    use crate::ui::widgets::{NumberFormat, SliderSpec};
    let lang = app.lang;
    let (y, h) = (bar.top() + 6.0, bar.height() - 12.0);
    let mut x = x + 4.0;
    let key = crate::shortcuts::menu_key("object.snap_toggle");
    let tip = match key {
        Some(k) => lang.pick(
            format!("スナップを常にする（{k}）。Ctrl を押している間は逆"),
            format!("Always snap ({k}). Hold Ctrl for the opposite"),
        ),
        None => lang
            .pick(
                "スナップを常にする。Ctrl を押している間は逆",
                "Always snap. Hold Ctrl for the opposite",
            )
            .to_owned(),
    };
    let label = lang.pick("スナップ", "Snap");
    let width = 23.0 + w::text_width(ui.painter(), label, t::LABEL) + 8.0;
    let on = w::toggle(
        ui,
        Rect::from_min_size(pos2(x, y), vec2(width, h)),
        "options.snap",
        label,
        app.objects.snap,
        Some(&tip),
        true,
    );
    if on != app.objects.snap {
        app.apply(Action::Object(crate::objects::ObjectAction::ToggleSnap));
    }
    x += width + 8.0;
    let steps = &mut app.objects.steps;
    for (id, name, value, min, max, format) in [
        (
            "options.snap.move",
            lang.pick("移動", "Move"),
            &mut steps.movement,
            0.001,
            100.0,
            NumberFormat {
                decimals: 3,
                trim: true,
                suffix: "",
            },
        ),
        (
            "options.snap.rotate",
            lang.pick("回転", "Rotate"),
            &mut steps.rotation,
            0.1,
            90.0,
            NumberFormat {
                decimals: 1,
                trim: true,
                suffix: "°",
            },
        ),
        (
            "options.snap.scale",
            lang.pick("拡縮", "Scale"),
            &mut steps.scale,
            0.001,
            10.0,
            NumberFormat {
                decimals: 3,
                trim: true,
                suffix: "",
            },
        ),
    ] {
        let r = Rect::from_min_size(pos2(x, y), vec2(110.0, h));
        if r.right() > bar.right() - 60.0 {
            break;
        }
        let out = w::slider(ui, r, id, *value, &SliderSpec::new(name, min, max, format));
        if out.changed {
            *value = out.value;
        }
        x += 118.0;
    }
}

/// 編集・ポーズのモードで暗くするドックのタブ（サブツール・ツールプロパティ・ブラシサイズと、色・カラーセット）。
pub fn dims_tab(tab: crate::Tab) -> bool {
    matches!(
        tab,
        crate::Tab::SubTools
            | crate::Tab::ToolProperties
            | crate::Tab::BrushSize
            | crate::Tab::Color
            | crate::Tab::ColorSets
    )
}

/// 編集・ポーズのモードの間、暗くした欄の上に理由のツールチップを出す（中の部品は押せない）。
pub fn dimmed_reason(ui: &mut Ui, app: &AppState, rect: Rect, salt: &'static str) {
    ui.interact(rect, ui.id().with(("mode.dimmed", salt)), Sense::hover())
        .on_hover_text(paint_only(app.lang));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Tool;
    use crate::view3d::pose::PoseAction;

    fn with_rig() -> AppState {
        let mut app = AppState::new(64, 64);
        app.apply(Action::Pose(PoseAction::LoadFigure));
        assert!(app.view3d.pose.session.is_some(), "{}", app.message);
        app
    }

    fn begin_stroke(app: &mut AppState) {
        let layer = app.selected_layer.unwrap();
        let settings = app.stroke_settings(false);
        let stroke = app.doc.begin_stroke(layer, &settings).unwrap();
        app.stroke = Some(stroke);
        app.canvas.stroke = Some(crate::state::StrokeSource::Mouse);
    }

    fn end_stroke(app: &mut AppState) {
        let stroke = app.stroke.take().unwrap();
        app.doc.cancel_stroke(stroke);
        app.canvas.stroke = None;
    }

    #[test]
    fn the_mode_starts_in_paint_and_each_mode_has_a_name_an_icon_and_a_command() {
        let app = AppState::new(32, 32);
        assert_eq!(app.mode, EditorMode::Paint);
        assert_eq!(app.edit_tool, EditTool::Select);
        for mode in EditorMode::ALL {
            for lang in Lang::ALL {
                assert!(!mode.name(lang).is_empty());
            }
            assert!(crate::commands::find(mode.command()).is_some(), "{mode:?}");
            assert_eq!(
                crate::commands::find(mode.command()).and_then(|c| c.runnable()),
                Some(Action::Mode(ModeAction::Set(mode)))
            );
        }
        assert!(EditorMode::Paint.paints());
        assert!(!EditorMode::Edit.paints() && !EditorMode::Pose.paints());
    }

    #[test]
    fn pose_mode_needs_a_model_with_bones() {
        let mut app = AppState::new(32, 32);
        assert!(!app.set_mode(EditorMode::Pose));
        assert_eq!(app.mode, EditorMode::Paint);
        assert_eq!(
            app.message,
            "ポーズのモードにできません（ボーンのあるモデルがありません）。"
        );
        // 編集はモデルが無くても入れる
        assert!(app.set_mode(EditorMode::Edit));
        assert_eq!(app.mode, EditorMode::Edit);
        let mut app = with_rig();
        assert!(app.set_mode(EditorMode::Pose));
        assert_eq!(app.mode, EditorMode::Pose);
        // ボーンが無くなったら（ほかのモデル）、フレームの初めにペイントへ
        app.view3d.pose.session = None;
        leave_pose_without_rig(&mut app);
        assert_eq!(app.mode, EditorMode::Paint);
    }

    #[test]
    fn the_mode_does_not_change_while_drawing() {
        let mut app = with_rig();
        begin_stroke(&mut app);
        for mode in [EditorMode::Edit, EditorMode::Pose] {
            app.apply(Action::Mode(ModeAction::Set(mode)));
            assert_eq!(app.mode, EditorMode::Paint);
            assert_eq!(app.message, crate::lang::refusals::during_stroke(app.lang));
        }
        // メニューとドロップダウンの項目も押せない（理由つき）
        for entry in entries(&app) {
            if let Entry::Item {
                enabled, action, ..
            } = entry
            {
                assert_eq!(
                    enabled,
                    action == Action::Mode(ModeAction::Set(EditorMode::Paint)),
                    "{action:?}"
                );
            }
        }
        end_stroke(&mut app);
        app.apply(Action::Mode(ModeAction::Set(EditorMode::Edit)));
        assert_eq!(app.mode, EditorMode::Edit);
        // 描けないモードからも、描いている間（ポーズのギズモはストロークではない）でなければ戻れる
        app.apply(Action::Mode(ModeAction::Set(EditorMode::Paint)));
        assert_eq!(app.mode, EditorMode::Paint);
    }

    #[test]
    fn choosing_a_drawing_tool_returns_to_paint_mode() {
        let mut app = with_rig();
        for mode in [EditorMode::Edit, EditorMode::Pose] {
            assert!(app.set_mode(mode));
            app.apply(Action::SelectTool(Tool::Eraser));
            assert_eq!(app.mode, EditorMode::Paint, "{mode:?}");
            assert_eq!(app.tool, Tool::Eraser);
            app.apply(Action::SelectTool(Tool::Brush));
        }
    }

    #[test]
    fn the_pose_toggle_and_the_edit_tools_go_through_the_mode() {
        let mut app = with_rig();
        app.apply(Action::Pose(PoseAction::ToggleMode));
        assert_eq!(app.mode, EditorMode::Pose);
        app.apply(Action::Pose(PoseAction::ToggleMode));
        assert_eq!(app.mode, EditorMode::Paint);
        // 編集のモードからポーズの切り替えを押すと、ポーズへ
        app.set_mode(EditorMode::Edit);
        app.apply(Action::Pose(PoseAction::ToggleMode));
        assert_eq!(app.mode, EditorMode::Pose);
        for tool in EditTool::ALL {
            app.apply(Action::Mode(ModeAction::Tool(tool)));
            assert_eq!(app.edit_tool, tool);
            assert!(!tool.name(Lang::Ja).is_empty() && !tool.name(Lang::En).is_empty());
        }
        assert_eq!(
            app.mode,
            EditorMode::Pose,
            "ツールの帯のツールはモードを替えない"
        );
    }

    #[test]
    fn leaving_paint_mode_ends_the_unfinished_shapes_of_the_paint_tools() {
        let mut app = AppState::new(64, 64);
        app.apply(Action::SelectTool(Tool::Polygon));
        app.sel.polygon.push((1.0, 1.0));
        assert!(app.sel_has_polygon_point());
        assert!(app.set_mode(EditorMode::Edit));
        assert!(!app.sel_has_polygon_point(), "多角形の途中の点を捨てた");
        assert_eq!(app.tool, Tool::Polygon, "ツールは替えない");
    }

    #[test]
    fn the_right_button_opens_the_island_menu_only_for_the_polygon_fill_tool_in_paint_mode() {
        let mut app = with_rig();
        app.tool = Tool::PolygonFill;
        assert!(app.right_opens_island_menu());
        for mode in [EditorMode::Edit, EditorMode::Pose] {
            app.set_mode(mode);
            assert!(!app.right_opens_island_menu(), "{mode:?}: スポイト");
        }
        app.set_mode(EditorMode::Paint);
        app.tool = Tool::Brush;
        assert!(!app.right_opens_island_menu());
    }

    #[test]
    fn the_mode_entries_mark_the_current_mode_and_explain_a_missing_rig() {
        let mut app = AppState::new(32, 32);
        for lang in Lang::ALL {
            app.lang = lang;
            let entries = entries(&app);
            assert_eq!(entries.len(), 3);
            for (entry, mode) in entries.iter().zip(EditorMode::ALL) {
                let Entry::Item {
                    label,
                    enabled,
                    check,
                    tooltip,
                    ..
                } = entry
                else {
                    panic!("項目");
                };
                assert_eq!(label, mode.name(lang));
                assert_eq!(
                    *check,
                    crate::ui::menu::Check::Icon {
                        name: mode.icon(),
                        on: mode == EditorMode::Paint
                    }
                );
                assert_eq!(*enabled, mode != EditorMode::Pose);
                assert_eq!(
                    tooltip.as_deref(),
                    (mode == EditorMode::Pose).then(|| no_rig(lang))
                );
            }
        }
    }
}
