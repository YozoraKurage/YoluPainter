//! テストの運転の口。テストは AppState の欄を直に触らず、この口を通す（`h.doc()`・`h.doc_mut()`・`h.act(…)`）。AppState の欄の置き場が
//! 変わっても（状態を切り分けるなど）、直すのはこのファイルだけで、テストの本文は変わらない。画面のテスト（`Harness<YoluApp>`）にも、
//! 画面なしのテスト（`AppState`）にも付く。
//!
//! - 操作は、`Action` があれば `act` で当てる（使う人の操作と同じ道を通る）。
//! - 欄の口は名前だけの口（読む `x()` と書く `x_mut()`。小さな値は `x()` が値を返し、`set_x(v)` で書く）。
//! - `app()`・`app_mut()` は AppState のメソッドを呼ぶため（`h.app().is_stroking()` など）にだけ使い、欄には使わない。
//! - モードの切り替えは確かめ付きの `AppState::set_mode`（`h.app_mut().set_mode(…)`）を使う（`set_mode` の欄の口は作らない）。
#![allow(dead_code)]

use egui_kittest::Harness;
use yolu_app::state::{Action, AppState};
use yolu_app::YoluApp;

pub trait Drive {
    /// AppState（メソッドを呼ぶため。欄は下の口を通す）。
    fn app(&self) -> &AppState;
    fn app_mut(&mut self) -> &mut AppState;

    /// 操作を当てる（`AppState::apply`）。
    fn act(&mut self, action: Action) {
        self.app_mut().apply(action);
    }

    /// 今の知らせの文（`AppState::message`）。
    fn message(&self) -> &str {
        &self.app().message
    }
    fn message_mut(&mut self) -> &mut String {
        &mut self.app_mut().message
    }

    // ───────── 欄の口（AppState の欄の並びの順）─────────
    fn lang(&self) -> yolu_app::lang::Lang {
        self.app().lang
    }
    fn set_lang(&mut self, value: yolu_app::lang::Lang) {
        self.app_mut().lang = value;
    }
    fn m2(&self) -> &yolu_app::m2::M2State {
        &self.app().m2
    }
    fn m2_mut(&mut self) -> &mut yolu_app::m2::M2State {
        &mut self.app_mut().m2
    }
    fn mat(&self) -> &yolu_app::matpaint::MaterialPaint {
        &self.app().mat
    }
    fn mat_mut(&mut self) -> &mut yolu_app::matpaint::MaterialPaint {
        &mut self.app_mut().mat
    }
    fn region(&self) -> &yolu_app::region::RegionState {
        &self.app().region
    }
    fn region_mut(&mut self) -> &mut yolu_app::region::RegionState {
        &mut self.app_mut().region
    }
    fn subtools(&self) -> &yolu_app::subtool::SubToolState {
        &self.app().subtools
    }
    fn subtools_mut(&mut self) -> &mut yolu_app::subtool::SubToolState {
        &mut self.app_mut().subtools
    }
    fn ramp_sets(&self) -> &yolu_app::rampsets::RampSets {
        &self.app().ramp_sets
    }
    fn ramp_sets_mut(&mut self) -> &mut yolu_app::rampsets::RampSets {
        &mut self.app_mut().ramp_sets
    }
    fn doc(&self) -> &yolu_app::engine::Document {
        &self.app().session.doc
    }
    fn doc_mut(&mut self) -> &mut yolu_app::engine::Document {
        &mut self.app_mut().session.doc
    }
    fn stroke(&self) -> &Option<yolu_app::engine::Stroke> {
        &self.app().session.stroke
    }
    fn stroke_mut(&mut self) -> &mut Option<yolu_app::engine::Stroke> {
        &mut self.app_mut().session.stroke
    }
    fn selected_layer(&self) -> Option<yolu_app::engine::LayerId> {
        self.app().session.selected_layer
    }
    fn set_selected_layer(&mut self, value: Option<yolu_app::engine::LayerId>) {
        self.app_mut().session.selected_layer = value;
    }
    fn tool(&self) -> yolu_app::state::Tool {
        self.app().tool
    }
    fn set_tool(&mut self, value: yolu_app::state::Tool) {
        self.app_mut().tool = value;
    }
    fn brush(&self) -> &yolu_app::state::BrushState {
        &self.app().brush
    }
    fn brush_mut(&mut self) -> &mut yolu_app::state::BrushState {
        &mut self.app_mut().brush
    }
    fn color(&self) -> &yolu_app::state::ColorState {
        &self.app().color
    }
    fn color_mut(&mut self) -> &mut yolu_app::state::ColorState {
        &mut self.app_mut().color
    }
    fn colorsets(&self) -> &yolu_app::colorsets::ColorSets {
        &self.app().colorsets
    }
    fn colorsets_mut(&mut self) -> &mut yolu_app::colorsets::ColorSets {
        &mut self.app_mut().colorsets
    }
    fn view(&self) -> &yolu_app::canvas::view::ViewState {
        &self.app().view
    }
    fn view_mut(&mut self) -> &mut yolu_app::canvas::view::ViewState {
        &mut self.app_mut().view
    }
    fn last_notice(&self) -> &Option<yolu_app::notice::Notice> {
        &self.app().last_notice
    }
    fn last_notice_mut(&mut self) -> &mut Option<yolu_app::notice::Notice> {
        &mut self.app_mut().last_notice
    }
    fn notice_log(&self) -> &yolu_app::notice::NoticeLog {
        &self.app().notice_log
    }
    fn notice_log_mut(&mut self) -> &mut yolu_app::notice::NoticeLog {
        &mut self.app_mut().notice_log
    }
    fn log_view(&self) -> &yolu_app::panels::log::LogView {
        &self.app().ui.log_view
    }
    fn log_view_mut(&mut self) -> &mut yolu_app::panels::log::LogView {
        &mut self.app_mut().ui.log_view
    }
    fn toast(&self) -> &yolu_app::toast::Toast {
        &self.app().ui.toast
    }
    fn toast_mut(&mut self) -> &mut yolu_app::toast::Toast {
        &mut self.app_mut().ui.toast
    }
    fn usage(&self) -> &yolu_app::usage::Usage {
        &self.app().usage
    }
    fn usage_mut(&mut self) -> &mut yolu_app::usage::Usage {
        &mut self.app_mut().usage
    }
    fn ui(&self) -> &yolu_app::state::UiState {
        &self.app().ui
    }
    fn ui_mut(&mut self) -> &mut yolu_app::state::UiState {
        &mut self.app_mut().ui
    }
    fn canvas(&self) -> &yolu_app::state::CanvasInput {
        &self.app().canvas
    }
    fn canvas_mut(&mut self) -> &mut yolu_app::state::CanvasInput {
        &mut self.app_mut().canvas
    }
    fn popup(&self) -> &Option<yolu_app::state::OpenPopup> {
        &self.app().ui.popup
    }
    fn popup_mut(&mut self) -> &mut Option<yolu_app::state::OpenPopup> {
        &mut self.app_mut().ui.popup
    }
    fn project_name(&self) -> &String {
        &self.app().session.project_name
    }
    fn project_name_mut(&mut self) -> &mut String {
        &mut self.app_mut().session.project_name
    }
    fn save_folder(&self) -> &Option<std::path::PathBuf> {
        &self.app().session.save_folder
    }
    fn save_folder_mut(&mut self) -> &mut Option<std::path::PathBuf> {
        &mut self.app_mut().session.save_folder
    }
    fn places(&self) -> &yolu_app::dialog::places::Places {
        &self.app().places
    }
    fn places_mut(&mut self) -> &mut yolu_app::dialog::places::Places {
        &mut self.app_mut().places
    }
    fn modified(&self) -> bool {
        self.app().session.is_modified()
    }
    fn set_modified(&mut self, value: bool) {
        if value {
            self.app_mut().session.mark_modified();
        } else {
            self.app_mut().session.clear_modified();
        }
    }
    fn rewritten_sets(&self) -> usize {
        self.app().session.rewritten_sets
    }
    fn set_rewritten_sets(&mut self, value: usize) {
        self.app_mut().session.rewritten_sets = value;
    }
    fn reset_layout(&self) -> bool {
        self.app().ui.reset_layout
    }
    fn set_reset_layout(&mut self, value: bool) {
        self.app_mut().ui.reset_layout = value;
    }
    fn quit(&self) -> bool {
        self.app().quit
    }
    fn set_quit(&mut self, value: bool) {
        self.app_mut().quit = value;
    }
    fn sets(&self) -> &yolu_app::sets::TextureSets {
        &self.app().session.sets
    }
    fn sets_mut(&mut self) -> &mut yolu_app::sets::TextureSets {
        &mut self.app_mut().session.sets
    }
    fn model(&self) -> &Option<yolu_app::model::SceneModel> {
        &self.app().session.model
    }
    fn model_mut(&mut self) -> &mut Option<yolu_app::model::SceneModel> {
        &mut self.app_mut().session.model
    }
    fn link(&self) -> &yolu_app::livelink::LinkView {
        &self.app().link
    }
    fn link_mut(&mut self) -> &mut yolu_app::livelink::LinkView {
        &mut self.app_mut().link
    }
    fn link_request(&self) -> &Option<yolu_app::livelink::LinkRequest> {
        &self.app().link_request
    }
    fn link_request_mut(&mut self) -> &mut Option<yolu_app::livelink::LinkRequest> {
        &mut self.app_mut().link_request
    }
    fn link_target(&self) -> &Option<yolu_app::livelink::LinkTarget> {
        &self.app().link_target
    }
    fn link_target_mut(&mut self) -> &mut Option<yolu_app::livelink::LinkTarget> {
        &mut self.app_mut().link_target
    }
    fn link_reopen(&self) -> &Option<yolu_protocol::files::Request> {
        &self.app().link_reopen
    }
    fn link_reopen_mut(&mut self) -> &mut Option<yolu_protocol::files::Request> {
        &mut self.app_mut().link_reopen
    }
    fn project_epoch(&self) -> u64 {
        self.app().session.project_epoch()
    }
    fn ops(&self) -> &yolu_app::mcp_server::OpsView {
        &self.app().ops
    }
    fn ops_mut(&mut self) -> &mut yolu_app::mcp_server::OpsView {
        &mut self.app_mut().ops
    }
    fn link_originals(&self) -> &yolu_app::livelink::OriginalMarks {
        &self.app().link_originals
    }
    fn link_originals_mut(&mut self) -> &mut yolu_app::livelink::OriginalMarks {
        &mut self.app_mut().link_originals
    }
    fn resolution_chosen(&self) -> bool {
        self.app().session.resolution_chosen
    }
    fn set_resolution_chosen(&mut self, value: bool) {
        self.app_mut().session.resolution_chosen = value;
    }
    fn project(&self) -> &Option<yolu_app::project::ProjectFile> {
        &self.app().session.project
    }
    fn project_mut(&mut self) -> &mut Option<yolu_app::project::ProjectFile> {
        &mut self.app_mut().session.project
    }
    fn dialog_request(&self) -> &Option<yolu_app::state::DialogRequest> {
        &self.app().ui.dialog_request
    }
    fn dialog_request_mut(&mut self) -> &mut Option<yolu_app::state::DialogRequest> {
        &mut self.app_mut().ui.dialog_request
    }
    fn view3d(&self) -> &yolu_app::view3d::View3dState {
        &self.app().view3d
    }
    fn view3d_mut(&mut self) -> &mut yolu_app::view3d::View3dState {
        &mut self.app_mut().view3d
    }
    fn clone(&self) -> &yolu_app::clone_source::CloneState {
        &self.app().clone
    }
    fn clone_mut(&mut self) -> &mut yolu_app::clone_source::CloneState {
        &mut self.app_mut().clone
    }
    fn shelf(&self) -> &yolu_app::shelf::ShelfState {
        &self.app().shelf
    }
    fn shelf_mut(&mut self) -> &mut yolu_app::shelf::ShelfState {
        &mut self.app_mut().shelf
    }
    fn library(&self) -> &yolu_app::library::LibraryState {
        &self.app().library
    }
    fn library_mut(&mut self) -> &mut yolu_app::library::LibraryState {
        &mut self.app_mut().library
    }
    fn sel(&self) -> &yolu_app::selection::SelState {
        &self.app().sel
    }
    fn sel_mut(&mut self) -> &mut yolu_app::selection::SelState {
        &mut self.app_mut().sel
    }
    fn bake(&self) -> &yolu_app::bake::BakeState {
        &self.app().bake
    }
    fn bake_mut(&mut self) -> &mut yolu_app::bake::BakeState {
        &mut self.app_mut().bake
    }
    fn export(&self) -> &yolu_app::export::ExportState {
        &self.app().export
    }
    fn export_mut(&mut self) -> &mut yolu_app::export::ExportState {
        &mut self.app_mut().export
    }
    fn psd(&self) -> &yolu_app::psd::PsdState {
        &self.app().psd
    }
    fn psd_mut(&mut self) -> &mut yolu_app::psd::PsdState {
        &mut self.app_mut().psd
    }
    fn distribute(&self) -> &yolu_app::distribute::DistributeState {
        &self.app().distribute
    }
    fn distribute_mut(&mut self) -> &mut yolu_app::distribute::DistributeState {
        &mut self.app_mut().distribute
    }
    fn save(&self) -> &yolu_app::project::SaveState {
        &self.app().save
    }
    fn save_mut(&mut self) -> &mut yolu_app::project::SaveState {
        &mut self.app_mut().save
    }
    fn stencil(&self) -> &yolu_app::stencil::StencilState {
        &self.app().stencil
    }
    fn stencil_mut(&mut self) -> &mut yolu_app::stencil::StencilState {
        &mut self.app_mut().stencil
    }
    fn fx(&self) -> &yolu_app::fx::FxState {
        &self.app().fx
    }
    fn fx_mut(&mut self) -> &mut yolu_app::fx::FxState {
        &mut self.app_mut().fx
    }
    fn np(&self) -> &yolu_app::newproject::NpState {
        &self.app().np
    }
    fn np_mut(&mut self) -> &mut yolu_app::newproject::NpState {
        &mut self.app_mut().np
    }
    fn layer_ops(&self) -> &yolu_app::layerops::LayerOpsState {
        &self.app().layer_ops
    }
    fn layer_ops_mut(&mut self) -> &mut yolu_app::layerops::LayerOpsState {
        &mut self.app_mut().layer_ops
    }
    fn transform(&self) -> &yolu_app::transform::TransformState {
        &self.app().transform
    }
    fn transform_mut(&mut self) -> &mut yolu_app::transform::TransformState {
        &mut self.app_mut().transform
    }
    fn eyedrop(&self) -> &yolu_app::eyedrop::EyedropState {
        &self.app().eyedrop
    }
    fn eyedrop_mut(&mut self) -> &mut yolu_app::eyedrop::EyedropState {
        &mut self.app_mut().eyedrop
    }
    fn path(&self) -> &yolu_app::pathtool::PathState {
        &self.app().path
    }
    fn path_mut(&mut self) -> &mut yolu_app::pathtool::PathState {
        &mut self.app_mut().path
    }
    fn fillfx(&self) -> &yolu_app::fillfx::FillFxState {
        &self.app().fillfx
    }
    fn fillfx_mut(&mut self) -> &mut yolu_app::fillfx::FillFxState {
        &mut self.app_mut().fillfx
    }
    fn gradient(&self) -> &yolu_app::gradient::GradientState {
        &self.app().gradient
    }
    fn gradient_mut(&mut self) -> &mut yolu_app::gradient::GradientState {
        &mut self.app_mut().gradient
    }
    fn drafting(&self) -> &yolu_app::drafting::Drafting {
        &self.app().drafting
    }
    fn drafting_mut(&mut self) -> &mut yolu_app::drafting::Drafting {
        &mut self.app_mut().drafting
    }
    fn rulers(&self) -> &yolu_app::rulers::RulerState {
        &self.app().rulers
    }
    fn rulers_mut(&mut self) -> &mut yolu_app::rulers::RulerState {
        &mut self.app_mut().rulers
    }
    fn update(&self) -> &yolu_app::update::UpdateState {
        &self.app().update
    }
    fn update_mut(&mut self) -> &mut yolu_app::update::UpdateState {
        &mut self.app_mut().update
    }
    fn prefs(&self) -> &yolu_app::prefs::PrefsState {
        &self.app().prefs
    }
    fn prefs_mut(&mut self) -> &mut yolu_app::prefs::PrefsState {
        &mut self.app_mut().prefs
    }
    fn uv_wireframe(&self) -> &yolu_app::uv_wireframe::Wireframe {
        &self.app().uv_wireframe
    }
    fn uv_wireframe_mut(&mut self) -> &mut yolu_app::uv_wireframe::Wireframe {
        &mut self.app_mut().uv_wireframe
    }
    fn uv_overlap(&self) -> &yolu_app::uv_wireframe::overlap::OverlapState {
        &self.app().uv_overlap
    }
    fn uv_overlap_mut(&mut self) -> &mut yolu_app::uv_wireframe::overlap::OverlapState {
        &mut self.app_mut().uv_overlap
    }
    fn shortcuts(&self) -> &yolu_app::shortcuts::ShortcutWindow {
        &self.app().ui.shortcuts
    }
    fn shortcuts_mut(&mut self) -> &mut yolu_app::shortcuts::ShortcutWindow {
        &mut self.app_mut().ui.shortcuts
    }
    fn pressure(&self) -> &yolu_app::pen::window::PressureWindow {
        &self.app().ui.pressure
    }
    fn pressure_mut(&mut self) -> &mut yolu_app::pen::window::PressureWindow {
        &mut self.app_mut().ui.pressure
    }
    fn pen_lost(&self) -> &Vec<u32> {
        &self.app().pen_lost
    }
    fn pen_lost_mut(&mut self) -> &mut Vec<u32> {
        &mut self.app_mut().pen_lost
    }
    fn clip(&self) -> &yolu_app::clipboard::ClipState {
        &self.app().clip
    }
    fn clip_mut(&mut self) -> &mut yolu_app::clipboard::ClipState {
        &mut self.app_mut().clip
    }
    fn brushes(&self) -> &yolu_app::brushes::BrushesState {
        &self.app().brushes
    }
    fn brushes_mut(&mut self) -> &mut yolu_app::brushes::BrushesState {
        &mut self.app_mut().brushes
    }
    fn crash(&self) -> &yolu_app::crash::window::Report {
        &self.app().crash
    }
    fn crash_mut(&mut self) -> &mut yolu_app::crash::window::Report {
        &mut self.app_mut().crash
    }
    fn recovery(&self) -> &yolu_app::recovery::RecoveryState {
        &self.app().recovery
    }
    fn recovery_mut(&mut self) -> &mut yolu_app::recovery::RecoveryState {
        &mut self.app_mut().recovery
    }
    fn toolset(&self) -> &yolu_app::toolset::ToolsetState {
        &self.app().toolset
    }
    fn toolset_mut(&mut self) -> &mut yolu_app::toolset::ToolsetState {
        &mut self.app_mut().toolset
    }
    fn automation(&self) -> &yolu_app::automation::Automation {
        &self.app().automation
    }
    fn automation_mut(&mut self) -> &mut yolu_app::automation::Automation {
        &mut self.app_mut().automation
    }
    fn text(&self) -> &yolu_app::textlayer::TextState {
        &self.app().text
    }
    fn text_mut(&mut self) -> &mut yolu_app::textlayer::TextState {
        &mut self.app_mut().text
    }
    fn mode(&self) -> yolu_app::mode::EditorMode {
        self.app().mode
    }
    fn edit_tool(&self) -> yolu_app::mode::EditTool {
        self.app().edit_tool
    }
    fn set_edit_tool(&mut self, value: yolu_app::mode::EditTool) {
        self.app_mut().edit_tool = value;
    }
    fn pie(&self) -> &yolu_app::pie::PieState {
        &self.app().pie
    }
    fn pie_mut(&mut self) -> &mut yolu_app::pie::PieState {
        &mut self.app_mut().pie
    }
    fn objects(&self) -> &yolu_app::objects::ObjectsState {
        &self.app().objects
    }
    fn objects_mut(&mut self) -> &mut yolu_app::objects::ObjectsState {
        &mut self.app_mut().objects
    }
    fn keys(&self) -> &yolu_app::keyconfig::KeyConfig {
        &self.app().keys
    }
    fn keys_mut(&mut self) -> &mut yolu_app::keyconfig::KeyConfig {
        &mut self.app_mut().keys
    }
    fn temp_tool(&self) -> &yolu_app::toolkeys::TempTool {
        &self.app().temp_tool
    }
    fn temp_tool_mut(&mut self) -> &mut yolu_app::toolkeys::TempTool {
        &mut self.app_mut().temp_tool
    }
    fn recent_projects(&self) -> &yolu_app::recent::Recent {
        &self.app().recent_projects
    }
    fn recent_projects_mut(&mut self) -> &mut yolu_app::recent::Recent {
        &mut self.app_mut().recent_projects
    }
    fn discard_ask(&self) -> &Option<yolu_app::discard::Ask> {
        &self.app().ui.discard_ask
    }
}

impl Drive for Harness<'_, YoluApp> {
    fn app(&self) -> &AppState {
        &self.state().state
    }
    fn app_mut(&mut self) -> &mut AppState {
        &mut self.state_mut().state
    }
}

impl Drive for AppState {
    fn app(&self) -> &AppState {
        self
    }
    fn app_mut(&mut self) -> &mut AppState {
        self
    }
}
