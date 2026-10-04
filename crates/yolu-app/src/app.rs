//! アプリの本体: 外枠（メニューバー・オプションバー・ツールの帯・ステータスバー）と、egui_dock のドッキング（Substance の並び。
//! 左: アセットとカラー、中央: キャンバスと 3D ビュー、右: テクスチャセットとレイヤーとプロパティ）、ポップアップ、3D ビューの知らせ、
//! Live Link（フレームの頭で受け、終わりに変わったタイルを出す）、ファイルの窓。

use std::collections::HashMap;

use egui::{pos2, vec2, Color32, Frame, Id, Rect, RichText, Ui, WidgetText};
use egui_dock::{DockArea, DockState, NodeIndex, TabViewer};

use crate::canvas::{self, display::CanvasDisplay};
use crate::livelink::LiveLink;
use crate::panels::{
    assets, color::ColorTextures, layers, layers::Thumbnails, properties, texture_sets,
    view3d::View3dHost, view3d::View3dSlot,
};
use crate::pen::{PenInput, PenSample};
use crate::shell::{self, MENU_TITLES};
use crate::state::{Action, AppState, DialogRequest, OpenPopup, PopupKind, DEFAULT_DOCUMENT_SIZE};
use crate::ui::fonts::{self, FontReport};
use crate::ui::menu::{self, PopupOutcome, PopupState};
use crate::ui::theme as t;
use crate::ui::{icons, widgets as w};
use crate::view3d::render::{View3dRenderer, View3dStats};

/// ドックのタブ。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tab {
    Assets,
    Color,
    Canvas,
    View3d,
    TextureSets,
    Layers,
    Properties,
}

impl Tab {
    pub fn title(self) -> &'static str {
        match self {
            Tab::Assets => "アセット",
            Tab::Color => "カラー",
            Tab::Canvas => "キャンバス",
            Tab::View3d => "3D ビュー",
            Tab::TextureSets => "テクスチャセット",
            Tab::Layers => "レイヤー",
            Tab::Properties => "プロパティ",
        }
    }
}

/// Substance Painter の並び（左: アセットとカラー、中央: キャンバスと 3D ビュー、右: 上からテクスチャセット・レイヤー・プロパティ）。
/// 左右の列は 1600 点の幅の窓で 300 点になる割合。egui_dock の割合は左（上）の子の取り分（分けた向きによらない）。
pub fn default_dock() -> DockState<Tab> {
    let mut dock = DockState::new(vec![Tab::Canvas, Tab::View3d]);
    let surface = dock.main_surface_mut();
    let [center, left] = surface.split_left(NodeIndex::root(), 0.19, vec![Tab::Assets]);
    let [_, right] = surface.split_right(center, 0.77, vec![Tab::TextureSets]);
    surface.split_below(left, 0.48, vec![Tab::Color]);
    let [_, layers] = surface.split_below(right, 0.24, vec![Tab::Layers]);
    surface.split_below(layers, 0.45, vec![Tab::Properties]);
    dock
}

/// ドックの見た目（Unity 版のパネルの見出し: 地は PanelHeader、選んだタブは PanelBg、境目は Border）。
fn dock_style(style: &egui::Style) -> egui_dock::Style {
    let mut s = egui_dock::Style::from_egui(style);
    s.dock_area_padding = None;
    s.main_surface_border_stroke = egui::Stroke::NONE;
    s.main_surface_border_rounding = egui::CornerRadius::ZERO;
    s.separator.width = 2.0;
    s.separator.extra = 2.0;
    s.separator.color_idle = t::BORDER;
    s.separator.color_hovered = t::ACCENT_DIM;
    s.separator.color_dragged = t::ACCENT;
    s.tab_bar.bg_fill = t::PANEL_HEADER;
    s.tab_bar.height = t::PANEL_HEADER_HEIGHT;
    s.tab_bar.hline_color = t::BORDER;
    s.tab_bar.corner_radius = egui::CornerRadius::ZERO;
    s.tab_bar.fill_tab_bar = false;
    let face = |bg: Color32, text: Color32| egui_dock::TabInteractionStyle {
        outline_color: Color32::TRANSPARENT,
        corner_radius: egui::CornerRadius::ZERO,
        bg_fill: bg,
        text_color: text,
    };
    s.tab.active = face(t::PANEL_BG, Color32::WHITE);
    s.tab.focused = face(t::PANEL_BG, Color32::WHITE);
    s.tab.active_with_kb_focus = face(t::PANEL_BG, Color32::WHITE);
    s.tab.focused_with_kb_focus = face(t::PANEL_BG, Color32::WHITE);
    s.tab.inactive = face(t::PANEL_HEADER, t::TEXT_DIM);
    s.tab.inactive_with_kb_focus = face(t::PANEL_HEADER, t::TEXT_DIM);
    s.tab.hovered = face(t::CONTROL_HOVER, t::TEXT);
    s.tab.hline_below_active_tab_name = false;
    s.tab.tab_body.inner_margin = egui::Margin::ZERO;
    s.tab.tab_body.stroke = egui::Stroke::NONE;
    s.tab.tab_body.corner_radius = egui::CornerRadius::ZERO;
    s.tab.tab_body.bg_fill = t::PANEL_BG;
    s.overlay.selection_color = t::ACCENT_SOFT;
    s
}

struct Tabs<'a> {
    app: &'a mut AppState,
    display: &'a mut CanvasDisplay,
    thumbs: &'a mut Thumbnails,
    colors: &'a mut ColorTextures,
    view3d: &'a mut View3dSlot,
    renderer3d: &'a mut Option<View3dRenderer>,
    pen: &'a [PenSample],
    tab_rects: HashMap<Tab, Rect>,
}

impl TabViewer for Tabs<'_> {
    type Tab = Tab;

    fn id(&mut self, tab: &mut Tab) -> Id {
        Id::new(("yolu-tab", *tab))
    }

    fn title(&mut self, tab: &mut Tab) -> WidgetText {
        RichText::new(tab.title()).font(t::HEADER.font()).into()
    }

    fn ui(&mut self, ui: &mut Ui, tab: &mut Tab) {
        // 選んだタブの上に青い線（Unity 版のパネルの見出しと同じ）
        if let Some(r) = self.tab_rects.get(tab) {
            let mut p = ui.painter().clone();
            p.set_clip_rect(*r);
            p.rect_filled(
                Rect::from_min_size(r.min, vec2(r.width(), 2.0)),
                0.0,
                t::ACCENT,
            );
        }
        match tab {
            Tab::Canvas => canvas::show(ui, self.app, self.display, self.pen),
            Tab::View3d => self
                .view3d
                .show(ui, self.app, self.renderer3d.as_mut(), self.pen),
            Tab::TextureSets => texture_sets::show(ui, self.app),
            Tab::Layers => layers::show(ui, self.app, self.thumbs),
            Tab::Color => crate::panels::color::show(ui, self.app, self.colors),
            Tab::Properties => properties::show(ui, self.app),
            Tab::Assets => assets::show(ui),
        }
    }

    fn on_tab_button(&mut self, tab: &mut Tab, response: &egui::Response) {
        self.tab_rects.insert(*tab, response.rect);
    }

    fn is_closeable(&self, _tab: &Tab) -> bool {
        false
    }

    fn scroll_bars(&self, _tab: &Tab) -> [bool; 2] {
        [false, false]
    }
}

/// アプリ。
pub struct YoluApp {
    pub state: AppState,
    pub dock: DockState<Tab>,
    display: CanvasDisplay,
    thumbs: Thumbnails,
    colors: ColorTextures,
    pen: PenInput,
    view3d: View3dSlot,
    /// 3D ビューの wgpu の描画（wgpu の装置が無ければ None）。
    renderer3d: Option<View3dRenderer>,
    pub fonts: FontReport,
    /// 最後のフレームのドックのタブのボタンの矩形（試験用。ドックのタブは読み上げの名前を持たない）。
    pub tab_rects: HashMap<Tab, Rect>,
    link: LiveLink,
    /// ファイルの窓・確かめの窓を開くか（eframe の窓だけ。試験では開かず、頼みを `state.dialog_request` に残す）。
    dialogs: bool,
    /// 終わると決めた（閉じる頼みを二度聞かない）。
    closing: bool,
}

impl YoluApp {
    /// 文脈に配色・書体・アイコンを入れる（窓を作るときに 1 度）。
    pub fn setup(ctx: &egui::Context) -> FontReport {
        t::apply(ctx);
        let report = fonts::install(ctx);
        icons::install(ctx);
        report
    }

    /// eframe の窓から作る（Windows ではペンの入力を窓に繋ぐ）。
    pub fn new(cc: &eframe::CreationContext<'_>) -> YoluApp {
        let fonts = Self::setup(&cc.egui_ctx);
        let pen = PenInput::attach(cc);
        let mut app = YoluApp::with_state(
            AppState::new(DEFAULT_DOCUMENT_SIZE, DEFAULT_DOCUMENT_SIZE),
            pen,
        )
        .with_render_state(cc.wgpu_render_state.as_ref());
        app.fonts = fonts;
        app.dialogs = true;
        // 3D ビューには、まず試しの立方体を出しておく（Live Link のモデルが来たら入れ替わる）
        app.state.view3d.load_demo();
        if app.pen.is_hooked() {
            app.state.message = "Windows Ink のペンを受けています。".into();
        }
        app
    }

    /// 文脈と状態から作る（試験用。配色・書体・アイコンも入れる）。
    pub fn for_context(ctx: &egui::Context, state: AppState, pen: PenInput) -> YoluApp {
        let fonts = Self::setup(ctx);
        let mut app = YoluApp::with_state(state, pen);
        app.fonts = fonts;
        app
    }

    /// 状態とペンの受け口から作る（`setup` は呼ぶ側で）。
    pub fn with_state(state: AppState, pen: PenInput) -> YoluApp {
        YoluApp {
            state,
            dock: default_dock(),
            display: CanvasDisplay::new(),
            thumbs: Thumbnails::default(),
            colors: ColorTextures::default(),
            pen,
            view3d: View3dSlot::default(),
            renderer3d: None,
            fonts: FontReport::default(),
            tab_rects: HashMap::new(),
            link: LiveLink::new(),
            dialogs: false,
            closing: false,
        }
    }

    pub fn link(&self) -> &LiveLink {
        &self.link
    }

    /// Live Link（試験でつなぎ先の名前を替える）。
    pub fn link_mut(&mut self) -> &mut LiveLink {
        &mut self.link
    }

    /// Live Link を始める・やめるの頼みと、ファイルの窓の頼みを当てる。
    fn handle_requests(&mut self, ctx: &egui::Context) {
        if let Some(request) = self.state.link_request.take() {
            self.link.request(request, ctx, &mut self.state);
            self.state.link = self.link.view();
        }
        if !self.dialogs {
            return;
        }
        match self.state.dialog_request.take() {
            Some(DialogRequest::New) => {
                if self.confirm_discard() {
                    self.state.apply(Action::NewProject);
                }
            }
            Some(DialogRequest::Open) => {
                if self.confirm_discard() {
                    if let Some(path) = rfd::FileDialog::new()
                        .set_title("プロジェクトを開く")
                        .add_filter("YoluPainter プロジェクト", &["ylp"])
                        .pick_file()
                    {
                        self.state.apply(Action::OpenProject(path));
                    }
                }
            }
            Some(DialogRequest::SaveAs) => {
                let name = format!("{}.ylp", self.state.project_name);
                if let Some(path) = rfd::FileDialog::new()
                    .set_title("別名で保存")
                    .add_filter("YoluPainter プロジェクト", &["ylp"])
                    .set_file_name(name)
                    .save_file()
                {
                    self.state.apply(Action::SaveProjectAs(path));
                }
            }
            None => {}
        }
    }

    /// 保存していない変更があっても終わってよいか（窓を開かない試験では聞かない）。
    fn confirm_close(&self) -> bool {
        if !self.state.modified || !self.dialogs {
            return true;
        }
        rfd::MessageDialog::new()
            .set_title("YoluPainter")
            .set_description("保存していない変更があります。変更を捨てて終わりますか？")
            .set_buttons(rfd::MessageButtons::YesNo)
            .set_level(rfd::MessageLevel::Warning)
            .show()
            == rfd::MessageDialogResult::Yes
    }

    /// 保存していない変更を捨ててよいか（窓を開かない試験では、聞かずに捨てる）。
    fn confirm_discard(&self) -> bool {
        if !self.state.modified || !self.dialogs {
            return true;
        }
        rfd::MessageDialog::new()
            .set_title("YoluPainter")
            .set_description("保存していない変更があります。変更を捨てますか？")
            .set_buttons(rfd::MessageButtons::YesNo)
            .set_level(rfd::MessageLevel::Warning)
            .show()
            == rfd::MessageDialogResult::Yes
    }

    /// 窓に落としたファイル（.ylp なら開く）。
    fn open_dropped(&mut self, ctx: &egui::Context) {
        let dropped = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .find(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("ylp")))
        });
        if let Some(path) = dropped {
            if self.state.is_stroking() {
                self.state.message = "描いている間は開きません。".into();
            } else if self.confirm_discard() {
                self.state.apply(Action::OpenProject(path));
            }
        }
    }

    /// 3D ビューを wgpu で描く（eframe・kittest の RenderState。None なら 3D は描けないと出す）。
    pub fn with_render_state(mut self, rs: Option<&eframe::egui_wgpu::RenderState>) -> YoluApp {
        self.renderer3d = rs.map(View3dRenderer::new);
        self
    }

    /// 最後に描いた 3D ビューの中身の表示域（画面の点。隠れていれば None）。
    pub fn view3d_rect(&self) -> Option<Rect> {
        self.view3d.content_rect()
    }

    /// 3D ビューの描画の数（上げたタイル・描いた回数。wgpu が無ければ None）。
    pub fn view3d_stats(&self) -> Option<View3dStats> {
        self.renderer3d.as_ref().map(|r| r.stats)
    }

    /// Live Link と同じ形のモデルを読む（Live Link が受けたときと同じ道: 記録・テクスチャセットの結び付け・3D の形。描いている
    /// 最中なら、3D の形は終わってから入れ替わる）。つながりの外から読んだものなので Unity には出さない。
    pub fn load_live_link_model(&mut self, model: &yolu_protocol::Model) -> Result<(), String> {
        self.state.receive_link_model(model, 0).1
    }

    /// Live Link と同じ形のポーズを当てる（描いている最中なら、終わってから）。
    pub fn apply_live_link_pose(&mut self, pose: &yolu_protocol::Pose) -> Result<(), String> {
        self.state.receive_link_pose(pose)
    }

    pub fn pen(&self) -> &PenInput {
        &self.pen
    }

    pub fn display(&self) -> &CanvasDisplay {
        &self.display
    }

    /// 最後に描いたキャンバスの表示域（画面の点）。
    pub fn canvas_view_rect(&self) -> Option<Rect> {
        self.state.canvas_rect
    }

    pub fn thumbnails(&self) -> &Thumbnails {
        &self.thumbs
    }

    /// 3D ビューの中身を描く外の窓の口を渡す。
    pub fn set_view3d_host(&mut self, host: Box<dyn View3dHost>) {
        self.view3d.set_host(host);
    }

    /// 1 フレーム（eframe と試験の両方がここを呼ぶ）。
    pub fn frame(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        self.state.popup_was_open = self.state.popup.is_some();
        let pen = self.pen.drain();
        shell::handle_shortcuts(&ctx, &mut self.state);
        self.open_dropped(&ctx);
        self.handle_requests(&ctx);
        self.link.poll(&mut self.state);
        self.state.link = self.link.view();
        // 3D ビューで描くマテリアル・隠すマテリアルを今のテクスチャセットに合わせる（ストロークが終わった後のフレームでも）
        self.state.sync_view3d();
        if self.state.reset_layout {
            self.dock = default_dock();
            self.state.reset_layout = false;
        }

        let mut bar = None;
        egui::Panel::top("yolu.menubar")
            .exact_size(t::MENU_BAR_HEIGHT)
            .frame(Frame::NONE)
            .show(ui, |ui| {
                let r = ui.max_rect();
                let open = match self.state.popup.as_ref().map(|p| p.kind) {
                    Some(PopupKind::MenuBar(i)) => Some(i),
                    _ => None,
                };
                bar = Some(menu::menu_bar(ui, r, &MENU_TITLES, open));
                // 右端: プロジェクトの名前と保存の状態
                let name = format!(
                    "{}{}",
                    self.state.project_name,
                    if self.state.modified { " •" } else { "" }
                );
                let title = Rect::from_min_max(
                    pos2(r.right() - 360.0, r.top()),
                    pos2(r.right() - 8.0, r.bottom()),
                );
                w::text(
                    ui.painter(),
                    title,
                    &name,
                    t::LABEL_DIM.with_color(if self.state.modified {
                        t::TEXT
                    } else {
                        t::TEXT_DIM
                    }),
                    w::Align::Right,
                );
            });
        egui::Panel::top("yolu.options")
            .exact_size(t::OPTIONS_BAR_HEIGHT)
            .frame(Frame::NONE)
            .show(ui, |ui| {
                let r = ui.max_rect();
                ui.add_enabled_ui(!self.state.is_stroking(), |ui| {
                    shell::options_bar(ui, &mut self.state, r)
                });
            });
        let uploaded = self.display.stats.total_tiles;
        egui::Panel::bottom("yolu.status")
            .exact_size(t::STATUS_BAR_HEIGHT)
            .frame(Frame::NONE)
            .show(ui, |ui| {
                let r = ui.max_rect();
                shell::status_bar(ui, &self.state, r, uploaded);
            });
        egui::Panel::left("yolu.tools")
            .exact_size(t::TOOL_STRIP_WIDTH)
            .frame(Frame::NONE)
            .resizable(false)
            .show(ui, |ui| {
                let r = ui.max_rect();
                shell::tool_strip(ui, &mut self.state, r);
            });
        self.view3d.begin_frame();
        egui::CentralPanel::default()
            .frame(Frame::NONE.fill(t::WINDOW_BG))
            .show(ui, |ui| {
                let style = dock_style(ui.style());
                // 浮かせた窓の閉じるボタンは出さない（タブは閉じない。閉じるとキャンバスを失う）。egui_dock 0.21 は代わりの名前を
                // 案内するが、その名前の関数はまだ無いので、古い名前を使う
                #[allow(deprecated)]
                let area = |dock| DockArea::new(dock).show_window_close_buttons(false);
                let mut tabs = Tabs {
                    app: &mut self.state,
                    display: &mut self.display,
                    thumbs: &mut self.thumbs,
                    colors: &mut self.colors,
                    view3d: &mut self.view3d,
                    renderer3d: &mut self.renderer3d,
                    pen: &pen,
                    tab_rects: HashMap::new(),
                };
                area(&mut self.dock)
                    .id(Id::new("yolu.dock"))
                    .style(style)
                    .show_close_buttons(false)
                    .show_add_buttons(false)
                    .show_leaf_collapse_buttons(false)
                    .show_leaf_close_all_buttons(false)
                    .show_inside(ui, &mut tabs);
                self.tab_rects = tabs.tab_rects;
            });

        let bar = bar.unwrap_or(menu::BarOutcome {
            rects: Vec::new(),
            pressed: None,
            hovered: None,
        });
        self.popups(&ctx, &bar);
        let popup_rect = self.state.popup.as_ref().map(|p| p.state.rect);
        self.view3d.end_frame(popup_rect);
        // メニューで選んだ Live Link・ファイルの頼みはこのフレームのうちに当て、描いた所を Unity へ出す
        self.handle_requests(&ctx);
        self.link.publish(&mut self.state);
        self.state.link = self.link.view();
        // 終了・窓を閉じる: 保存していない変更があれば聞く（窓を開かない試験では聞かない）
        let close_requested = ctx.input(|i| i.viewport().close_requested());
        if (self.state.quit || close_requested) && !self.closing {
            if self.confirm_close() {
                self.closing = true;
                if self.state.quit {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            } else {
                if close_requested {
                    ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                }
                self.state.quit = false;
            }
        }
    }

    fn open_bar_menu(&mut self, ctx: &egui::Context, bar: &menu::BarOutcome, index: usize) {
        if let Some(item) = bar.rects.get(index) {
            let anchor =
                Rect::from_min_size(pos2(item.left(), item.bottom()), vec2(item.width(), 0.0));
            self.state.popup = Some(OpenPopup {
                kind: PopupKind::MenuBar(index),
                state: PopupState::new(ctx, anchor),
            });
        }
    }

    fn popups(&mut self, ctx: &egui::Context, bar: &menu::BarOutcome) {
        let open_bar = match self.state.popup.as_ref().map(|p| p.kind) {
            Some(PopupKind::MenuBar(i)) => Some(i),
            _ => None,
        };
        // メニューバー: 押したら開く（開いている見出しなら閉じる）、開いているあいだは別の見出しへ移れば切り替える
        if let Some(i) = bar.pressed {
            if open_bar == Some(i) {
                self.state.popup = None;
            } else if !self.state.is_stroking() {
                self.open_bar_menu(ctx, bar, i);
            }
        } else if let (Some(open), Some(hover)) = (open_bar, bar.hovered) {
            if open != hover {
                self.open_bar_menu(ctx, bar, hover);
            }
        }
        let Some(mut open) = self.state.popup.take() else {
            return;
        };
        let entries = shell::popup_entries(&self.state, open.kind);
        let keep: Vec<Rect> = if matches!(open.kind, PopupKind::MenuBar(_)) {
            bar.rects.clone()
        } else {
            Vec::new()
        };
        match menu::show(ctx, Id::new("yolu.popup"), &mut open.state, &entries, &keep) {
            PopupOutcome::Open => self.state.popup = Some(open),
            PopupOutcome::Close => {}
            PopupOutcome::Chosen(action) => self.state.apply(action),
            PopupOutcome::Step(d) => {
                if let PopupKind::MenuBar(i) = open.kind {
                    let n = MENU_TITLES.len() as i32;
                    self.open_bar_menu(ctx, bar, (i as i32 + d).rem_euclid(n) as usize);
                } else {
                    self.state.popup = Some(open);
                }
            }
        }
    }

    /// 操作を当てる（試験・外から）。
    pub fn apply(&mut self, action: Action) {
        self.state.apply(action);
    }
}

impl eframe::App for YoluApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        self.frame(ui);
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        egui::Rgba::from(t::WINDOW_BG).to_array()
    }
}
