//! アプリの本体: 外枠（メニューバー・オプションバー・ツールの帯・ステータスバー）と、egui_dock のドッキング（Substance の並び。
//! 左: アセットとカラー、中央: キャンバスと 3D ビュー、右: テクスチャセットとレイヤーとプロパティ）、ポップアップ、3D ビューの知らせ、
//! Live Link（フレームの頭で受け、終わりに変わったタイルを出す）、ファイルの窓。

use std::collections::HashMap;

use egui::{pos2, vec2, Color32, Frame, Id, Rect, RichText, Sense, Ui, WidgetText};
use egui_dock::{DockArea, DockState, NodeIndex, TabViewer};

use crate::canvas::{self, display::CanvasDisplay};
use crate::livelink::LiveLink;
use crate::panels::{
    assets, color::ColorTextures, layers, layers::Thumbnails, properties, texture_sets,
    view3d::View3dHost, view3d::View3dSlot,
};
use crate::pen::{PenInput, PenSample};
use crate::settings::{Problem, Settings};
use crate::shell;
use crate::state::{Action, AppState, DialogRequest, OpenPopup, PopupKind, DEFAULT_DOCUMENT_SIZE};
use crate::ui::fonts;
use crate::ui::menu::{self, PopupOutcome, PopupState};
use crate::ui::theme as t;
use crate::ui::{icons, widgets as w};
use crate::view3d::render::{View3dRenderer, View3dStats};

/// ドックのタブ。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tab {
    /// ブラシの一覧・ツールプロパティ・ブラシサイズ（左のドックの先頭）。
    Brushes,
    Assets,
    Color,
    Canvas,
    View3d,
    TextureSets,
    Layers,
    Properties,
    Channels,
}

impl Tab {
    pub fn title(self) -> &'static str {
        self.title_in(crate::lang::Lang::Ja)
    }

    /// 言語ごとのタブの名前。
    pub fn title_in(self, lang: crate::lang::Lang) -> &'static str {
        match self {
            Tab::Brushes => lang.pick("ブラシ", "Brushes"),
            Tab::Assets => lang.pick("アセット", "Assets"),
            Tab::Color => lang.pick("カラー", "Color"),
            Tab::Canvas => lang.pick("キャンバス", "Canvas"),
            Tab::View3d => lang.pick("3D ビュー", "3D View"),
            Tab::TextureSets => lang.pick("テクスチャセット", "Texture Sets"),
            Tab::Layers => lang.pick("レイヤー", "Layers"),
            Tab::Properties => lang.pick("プロパティ", "Properties"),
            Tab::Channels => lang.pick("チャンネル", "Channels"),
        }
    }
}

/// Substance Painter の並び（左: ブラシとアセットとチャンネルとカラー、中央: キャンバスと 3D ビュー、右: 上からテクスチャセット・レイヤー・
/// プロパティ）。ブラシのパネルは一覧・ツールプロパティ・ブラシサイズが縦に入るので、左の列の上を高めに取る。
/// 左の列は、いちばん小さい窓（960 点）でも 3 つのタブ（英語の Brushes・Assets・Channels）の見出しが収まる割合（1600 点の窓で約 330 点）。
/// 右の列は 1600 点の幅の窓で約 300 点（左の列を広げた分、中の割合を減らして右の幅を前と同じにした）。egui_dock の割合は左（上）の子の取り分（分けた向きによらない）。
pub fn default_dock() -> DockState<Tab> {
    let mut dock = DockState::new(vec![Tab::Canvas, Tab::View3d]);
    let surface = dock.main_surface_mut();
    let [center, left] =
        surface.split_left(
        NodeIndex::root(),
        0.21,
        vec![Tab::Brushes, Tab::Assets, Tab::Channels],
    );
    let [_, right] = surface.split_right(center, 0.764, vec![Tab::TextureSets]);
    surface.split_below(left, 0.66, vec![Tab::Color]);
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
    /// このフレームにタブの見出しをつかんでいる（押している・動かしている・離した）か。
    grabbed: bool,
}

impl TabViewer for Tabs<'_> {
    type Tab = Tab;

    fn id(&mut self, tab: &mut Tab) -> Id {
        Id::new(("yolu-tab", *tab))
    }

    fn title(&mut self, tab: &mut Tab) -> WidgetText {
        RichText::new(tab.title_in(self.app.lang))
            .font(t::HEADER.font())
            .into()
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
            Tab::Channels => crate::panels::channels::show(ui, self.app),
            Tab::Layers => layers::show(ui, self.app, self.thumbs),
            Tab::Color => crate::panels::color::show(ui, self.app, self.colors),
            Tab::Properties => properties::show(ui, self.app),
            Tab::Assets => assets::show(ui, self.app),
            Tab::Brushes => crate::panels::brushes::show(ui, self.app),
        }
    }

    fn on_tab_button(&mut self, tab: &mut Tab, response: &egui::Response) {
        self.tab_rects.insert(*tab, response.rect);
        self.grabbed |=
            response.is_pointer_button_down_on() || response.dragged() || response.drag_stopped();
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
    /// サイドボタンを押したペンの接触を、egui の部品にも右ボタンとして届ける。
    pen_buttons: crate::pen::ButtonMap,
    view3d: View3dSlot,
    /// 3D ビューの wgpu の描画（wgpu の装置が無ければ None）。
    renderer3d: Option<View3dRenderer>,
    /// 最後のフレームのドックのタブのボタンの矩形（試験用。ドックのタブは読み上げの名前を持たない）。
    pub tab_rects: HashMap<Tab, Rect>,
    link: LiveLink,
    /// ファイルの窓・確かめの窓を開くか（eframe の窓だけ。試験では開かず、頼みを `state.dialog_request` に残す）。
    dialogs: bool,
    /// 終わると決めた（閉じる頼みを二度聞かない）。
    closing: bool,
    settings: Option<(std::path::PathBuf, Settings)>,
    /// 前のフレームでウィンドウにフォーカスがあったか（失ったら復旧の書き置きを待たずに書く）。
    was_focused: Option<bool>,
    /// 表示の合成の設定で、キャンバスの表示に入れた値（変わったときだけ入れ直す。試験や環境変数で決めた方針を、設定が変わらないうちは
    /// 上書きしない）。
    compositing_applied: crate::settings::Compositing,
}

impl YoluApp {
    /// 文脈に配色・書体・アイコンを入れる（窓を作るときに 1 度）。
    pub fn setup(ctx: &egui::Context) {
        t::apply(ctx);
        fonts::install(ctx);
        icons::install(ctx);
    }

    /// eframe の窓から作る（Windows ではペンの入力を窓に繋ぐ）。
    pub fn new(cc: &eframe::CreationContext<'_>) -> YoluApp {
        if let Some(rs) = &cc.wgpu_render_state {
            let info = rs.adapter.get_info();
            crate::crash::gpu(&info.name, &format!("{:?}", info.backend));
        }
        Self::setup(&cc.egui_ctx);
        let pen = PenInput::attach(cc);
        let mut app = YoluApp::with_settings(crate::settings::path(), pen)
            .with_render_state(cc.wgpu_render_state.as_ref());
        app.dialogs = true;
        if let Some(dir) = crate::crash::directory() {
            app.state.crash = crate::crash::window::Report::load(dir);
        }
        // ブラシの見本のストロークは別のスレッドで描く（取り込んだ大きな筆先でも画面が止まらない）。試験の窓は画面のスレッドで描く
        app.state.brushes.samples.render_in_background(&cc.egui_ctx);
        app.state.brushes.krita.load_in_background();
        // 本物の OS のクリップボード（画像のコピー・貼り付け）に繋ぐ。試験の窓は繋がない
        app.state.clip.use_system();
        // 3D ビューには、まず試しの立方体を出しておく（Live Link のモデルが来たら入れ替わる）
        app.state.view3d.load_demo();
        // 落ちた前の実行があれば復旧の窓を開く（起動の引数のプロジェクトより先に。どちらも開ける）
        app.start_recovery();
        // 関連付け（.ylp のダブルクリック）で起動されたら、そのプロジェクトを開く
        app.open_startup_project(std::env::args_os());
        // 更新: 初めてなら問いを出し、「確かめる」を選んでいれば確かめる（実際の窓だけ。試験は呼ばない）
        app.state.update_startup();
        app
    }

    /// 復旧を始める（設定のフォルダの下の置き場。前の実行が落ちていれば復旧の窓が開く）。始められなければ使わず、理由を状態の帯に出す。
    pub fn start_recovery(&mut self) {
        self.start_recovery_with(crate::recovery::RecoverySettings::path());
    }

    /// `start_recovery` の、復旧の設定のファイル（`recovery.conf`）の場所を渡せる形。
    pub fn start_recovery_with(&mut self, conf: Option<std::path::PathBuf>) {
        let lang = self.state.lang;
        let note = match self.state.recovery.start_from(conf) {
            Ok(problems) => problems.first().map(|p| lang.recovery_settings_problem(p)),
            Err(e) => Some(lang.recovery_unavailable(&e)),
        };
        if let Some(note) = note {
            if !self.state.message.is_empty() {
                self.state.message.push(' ');
            }
            self.state.message.push_str(&note);
        }
    }

    /// 起動の引数（実行ファイルの名前のあと）に .ylp があれば開く。開けないときは、`OpenProject` が message に理由を書く。
    pub fn open_startup_project(&mut self, args: impl Iterator<Item = std::ffi::OsString>) {
        if let Some(path) = startup_project(args) {
            self.state.apply(Action::OpenProject(path));
        }
    }

    /// 設定のファイル（無ければ保存しない）から言語・書き出しの余白・メモリの予算・退避を残す数などを決めて作る（`setup` は呼ぶ側で）。
    /// 最初のレイヤー・テクスチャセット・プロジェクトの名前がその言語になる。読めない設定・正しくない値は既定（日本語・自動の予算・
    /// すべて残す、など）に戻し、理由を知らせる
    /// （ファイルは、設定を選び直すまで触らない）。
    fn with_settings(settings: Option<std::path::PathBuf>, pen: PenInput) -> YoluApp {
        let (loaded, problems) = settings.as_deref().map(crate::settings::load).unwrap_or_default();
        let lang = loaded.lang;
        let mut app = YoluApp::with_state(
            AppState::new_in(DEFAULT_DOCUMENT_SIZE, DEFAULT_DOCUMENT_SIZE, lang),
            pen,
        );
        app.state.load_settings(loaded.clone());
        // 利用者のブラシは設定のフォルダの brushes/（読めないファイルは読み飛ばし、知らせる）
        if let Some(dir) = settings.as_deref().and_then(|p| p.parent()) {
            app.state.attach_brush_store(dir.join("brushes"));
        }
        let mut notices: Vec<String> = Vec::new();
        notices.extend(startup_message(lang, &problems));
        notices.extend(app.state.brush_problem_message());
        if !notices.is_empty() {
            app.state.message = notices.join(" ");
        }
        // 「起動時に更新を確かめる」の選択は、言語の設定と同じフォルダの別のファイル
        if let Some(path) = settings.as_deref().and_then(crate::update::config::path_for) {
            app.state.update.attach_config(path);
        }
        // 表示の合成の設定（自動のときは、環境変数 `YOLUPAINTER_CANVAS` か自動のまま）
        app.compositing_applied = loaded.compositing;
        if loaded.compositing != crate::settings::Compositing::Auto {
            app.display.set_backend(canvas_backend(loaded.compositing));
        }
        app.settings = settings.map(|path| (path, loaded));
        app
    }

    /// 設定の窓で表示の合成が変わっていれば、キャンバスの表示に入れる（次の合成から効く。保存・書き出しの合成は変わらず CPU）。
    fn apply_compositing(&mut self) {
        let now = self.state.prefs.settings.compositing;
        if now != self.compositing_applied {
            self.compositing_applied = now;
            self.display.set_backend(canvas_backend(now));
        }
    }

    /// 文脈と設定のファイルから作る（試験用。`for_context` に、設定の読み書きを足したもの）。
    pub fn for_context_with_settings(ctx: &egui::Context, settings: Option<std::path::PathBuf>, pen: PenInput) -> YoluApp {
        Self::setup(ctx);
        YoluApp::with_settings(settings, pen)
    }

    /// 設定（言語・書き出しの余白・メモリの予算・スレッド・合成・棚の場所・退避を残す数・選択範囲の帯）の選択が変わっていれば、設定のファイルに書く。
    /// 書けなくても動作は変えず、知らせるだけ。失敗しても同じ選択では再試行しない（毎フレームの I/O と、知らせの上書きを避ける）。
    /// 退避の数は、スライダーをドラッグしている間は書かない（離したとき、または Esc で戻した値が書いてある値と同じなら書かない）。
    fn persist_settings(&mut self) {
        let Some((path, saved)) = &mut self.settings else {
            return;
        };
        let mut now = self.state.settings();
        if self.state.prefs.dragging {
            now.backups = saved.backups;
        }
        if *saved == now {
            return;
        }
        *saved = now.clone();
        if crate::settings::save(path, &now).is_err() {
            self.state.message = now.lang.pick("設定を保存できません。", "Cannot save the settings.").into();
        }
    }

    /// 文脈と状態から作る（試験用。配色・書体・アイコンも入れる）。
    pub fn for_context(ctx: &egui::Context, state: AppState, pen: PenInput) -> YoluApp {
        Self::setup(ctx);
        YoluApp::with_state(state, pen)
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
            pen_buttons: crate::pen::ButtonMap::default(),
            view3d: View3dSlot::default(),
            renderer3d: None,
            tab_rects: HashMap::new(),
            link: LiveLink::new(),
            dialogs: false,
            closing: false,
            settings: None,
            was_focused: None,
            compositing_applied: crate::settings::Compositing::Auto,
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
        // 復旧の世代を開く頼み（今の変更を捨ててよいか聞いてから。窓を開かない試験では聞かない）
        if let Some(open) = self.state.recovery.take_open_request() {
            if self.confirm_discard() {
                self.state.recovery_open(open);
            }
        }
        if !self.dialogs {
            return;
        }
        match self.state.dialog_request.take() {
            Some(DialogRequest::New) => {
                // 保存していない変更は先に聞く（窓を開いてから聞くと、作業を捨てる前に窓の設定が無駄になる）
                if self.confirm_discard() {
                    self.state.apply(Action::Project(crate::newproject::NpAction::OpenNew));
                }
            }
            Some(DialogRequest::ProjectModel) => {
                let lang = self.state.lang;
                if let Some(path) = rfd::FileDialog::new()
                    .set_title(lang.pick("モデルを選ぶ", "Choose a model"))
                    .add_filter("FBX", &["fbx", "FBX"])
                    .pick_file()
                {
                    self.state
                        .apply(Action::Project(crate::newproject::NpAction::ChooseModel(path)));
                }
            }
            Some(DialogRequest::Open) => {
                let lang = self.state.lang;
                if self.confirm_discard() {
                    if let Some(path) = rfd::FileDialog::new()
                        .set_title(lang.pick("プロジェクトを開く", "Open Project"))
                        .add_filter(lang.pick("YoluPainter プロジェクト", "YoluPainter Project"), &["ylp"])
                        .pick_file()
                    {
                        self.state.apply(Action::OpenProject(path));
                    }
                }
            }
            Some(DialogRequest::SaveAs) => {
                let lang = self.state.lang;
                let name = format!("{}.ylp", self.state.project_name);
                if let Some(path) = rfd::FileDialog::new()
                    .set_title(lang.pick("別名で保存", "Save As"))
                    .add_filter(lang.pick("YoluPainter プロジェクト", "YoluPainter Project"), &["ylp"])
                    .set_file_name(name)
                    .save_file()
                {
                    self.state.apply(Action::SaveProjectAs(path));
                }
            }
            Some(DialogRequest::OpenModel) => {
                let lang = self.state.lang;
                if let Some(path) = rfd::FileDialog::new()
                    .set_title(lang.pick("3D ビューに FBX を開く", "Open FBX in the 3D View"))
                    .add_filter("FBX", &["fbx", "FBX"])
                    .pick_file()
                {
                    crate::view3d::pose::open_file(&mut self.state, &path);
                }
            }
            Some(
                request @ (DialogRequest::ShelfImport
                | DialogRequest::ShelfExport
                | DialogRequest::ShelfRemove),
            ) => assets::run_dialog(&mut self.state, request),
            Some(DialogRequest::ExportFolder(id)) => {
                let lang = self.state.lang;
                if let Some(dir) = rfd::FileDialog::new()
                    .set_title(
                        lang.pick("画像を書き出すフォルダ", "Folder for the exported images"),
                    )
                    .pick_folder()
                {
                    self.state
                        .apply(Action::Export(crate::export::ExportAction::TemplateTo {
                            id,
                            dir,
                        }));
                }
            }
            Some(DialogRequest::ExportChannel) => {
                let lang = self.state.lang;
                let mut dialog = rfd::FileDialog::new()
                    .set_title(lang.pick("チャンネルを PNG に書き出す", "Export the channel as PNG"))
                    .add_filter("PNG", &["png"])
                    .set_file_name(crate::export::default_channel_file_name(&self.state));
                if let Some(dir) = self
                    .state
                    .project
                    .as_ref()
                    .and_then(|p| p.path().parent())
                    .filter(|d| d.is_dir())
                {
                    dialog = dialog.set_directory(dir);
                }
                if let Some(path) = dialog.save_file() {
                    // 拡張子が無ければ .png を足す（窓の種類で付かない環境がある）。足した名前は窓が確かめていない
                    self.state
                        .apply(Action::Export(crate::export::channel_action(path)));
                }
            }
            Some(DialogRequest::PrefsLibraryFolder) => {
                let lang = self.state.lang;
                let mut dialog = rfd::FileDialog::new()
                    .set_title(lang.pick("棚の場所", "Library folder"));
                if let Some(current) = self.state.prefs.settings.library_folder().filter(|d| d.is_dir()) {
                    dialog = dialog.set_directory(current);
                }
                if let Some(dir) = dialog.pick_folder() {
                    self.state.apply(Action::Prefs(crate::prefs::PrefsAction::Set(
                        crate::prefs::Pref::LibraryFolder(Some(dir)),
                    )));
                }
            }
            Some(DialogRequest::ExportChannelsFolder) => {
                let lang = self.state.lang;
                if let Some(dir) = rfd::FileDialog::new()
                    .set_title(
                        lang.pick("画像を書き出すフォルダ", "Folder for the exported images"),
                    )
                    .pick_folder()
                {
                    self.state
                        .apply(Action::Export(crate::export::ExportAction::ChannelsTo(dir)));
                }
            }
            Some(DialogRequest::PsdImport(target)) => {
                let lang = self.state.lang;
                // 今の文書を替えるときは、保存していない変更を捨ててよいか聞く
                if target == crate::psd::PsdTarget::NewSet || self.confirm_discard() {
                    if let Some(path) = rfd::FileDialog::new()
                        .set_title(lang.pick("PSD を読み込む", "Import PSD"))
                        .add_filter("PSD", &["psd", "PSD"])
                        .pick_file()
                    {
                        self.state
                            .apply(Action::Psd(crate::psd::PsdAction::Import { path, target }));
                    }
                }
            }
            Some(DialogRequest::PsdExport) => {
                let lang = self.state.lang;
                let stem = crate::export::stem(&self.state);
                let name = if self.state.sets.len() > 1 {
                    format!("{stem}_{}.psd", self.state.sets.current().name)
                } else {
                    format!("{stem}.psd")
                };
                let mut dialog = rfd::FileDialog::new()
                    .set_title(lang.pick("PSD に書き出す", "Export PSD"))
                    .add_filter("PSD", &["psd"])
                    .set_file_name(name);
                if let Some(dir) = self.state.project.as_ref().and_then(|p| p.path().parent()) {
                    dialog = dialog.set_directory(dir);
                }
                if let Some(path) = dialog.save_file() {
                    self.state
                        .apply(Action::Psd(crate::psd::PsdAction::Export(path)));
                }
            }
            Some(DialogRequest::OpenStencil) => {
                let lang = self.state.lang;
                if let Some(path) = rfd::FileDialog::new()
                    .set_title(lang.pick("ステンシルの画像を開く", "Open a stencil image"))
                    .add_filter("PNG", &["png", "PNG"])
                    .pick_file()
                {
                    self.state
                        .apply(Action::Stencil(crate::stencil::StencilOp::Load(path)));
                }
            }
            Some(DialogRequest::FillImage) => {
                let lang = self.state.lang;
                if let Some(path) = rfd::FileDialog::new()
                    .set_title(lang.pick("画像を棚へ取り込む", "Add an image to the shelf"))
                    .add_filter("PNG", &["png", "PNG"])
                    .pick_file()
                {
                    self.state
                        .apply(Action::Fill(crate::fillfx::FillOp::ImportImage(path)));
                }
            }
            Some(DialogRequest::ImportBrushes) => {
                let lang = self.state.lang;
                if let Some(paths) = rfd::FileDialog::new()
                    .set_title(lang.pick("ブラシを取り込む", "Import Brushes"))
                    .add_filter(
                        lang.pick("ブラシのファイル", "Brush files"),
                        &yolu_io::brushes::FileKind::EXTENSIONS,
                    )
                    .pick_files()
                {
                    self.state
                        .apply(Action::Brush(crate::brushes::BrushAction::Import(paths)));
                }
            }
            None => {}
        }
    }

    /// 保存していない変更があっても終わってよいか（窓を開かない試験では聞かない）。
    fn confirm_close(&self) -> bool {
        // 更新のために終わるときは、保存するか捨てるかを更新の窓で選び済み
        if !self.state.modified || !self.dialogs || self.state.update.is_quitting() {
            return true;
        }
        rfd::MessageDialog::new()
            .set_title("YoluPainter")
            .set_description(self.state.lang.pick(
                "保存していない変更があります。変更を捨てて終わりますか？",
                "There are unsaved changes. Discard them and quit?",
            ))
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
            .set_description(self.state.lang.pick(
                "保存していない変更があります。変更を捨てますか？",
                "There are unsaved changes. Discard them?",
            ))
            .set_buttons(rfd::MessageButtons::YesNo)
            .set_level(rfd::MessageLevel::Warning)
            .show()
            == rfd::MessageDialogResult::Yes
    }

    /// 窓に落としたファイル（.ylp なら開く。ブラシのファイル（ABR・GBR・GIH・VBR・PAT）なら取り込む。PNG はブラシの一覧の上に
    /// 落としたときだけブラシの筆先として取り込む）。
    fn open_dropped(&mut self, ctx: &egui::Context) {
        let dropped: Vec<std::path::PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .collect()
        });
        if dropped.is_empty() {
            return;
        }
        let project = dropped
            .iter()
            .find(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("ylp")));
        if let Some(path) = project {
            if self.state.is_stroking() {
                self.state.message = self.state.lang.pick("描いている間は開きません。", "Cannot open during a stroke.").into();
            } else if self.confirm_discard() {
                self.state.apply(Action::OpenProject(path.clone()));
            }
            return;
        }
        let over_list = ctx
            .input(|i| i.pointer.latest_pos())
            .zip(self.state.brushes.ui.list_rect)
            .is_some_and(|(p, r)| r.contains(p));
        let brushes: Vec<std::path::PathBuf> = dropped
            .into_iter()
            .filter(|p| crate::brushes::import::is_brush_file(p))
            .filter(|p| over_list || !crate::brushes::import::is_png(p))
            .collect();
        if !brushes.is_empty() {
            self.state
                .apply(Action::Brush(crate::brushes::BrushAction::Import(brushes)));
        }
    }

    /// 3D ビューを wgpu で描く（eframe・kittest の RenderState。None なら 3D は描けないと出す）。
    pub fn with_render_state(mut self, rs: Option<&eframe::egui_wgpu::RenderState>) -> YoluApp {
        self.renderer3d = rs.map(View3dRenderer::new);
        // キャンバスの合成も同じ装置で（使えるときは GPU。使えなければ CPU の表示）
        self.display.attach_render_state(rs.cloned());
        self
    }

    /// キャンバスの表示の合成の方針（自動・GPU・CPU）。既定は環境変数 `YOLUPAINTER_CANVAS`、無ければ自動。
    pub fn set_canvas_backend(&mut self, policy: crate::canvas::gpu::CanvasBackend) {
        self.display.set_backend(policy);
    }

    /// キャンバスの表示の合成の方針（設定の「表示の合成」と環境変数で決まる）。
    pub fn canvas_backend(&self) -> crate::canvas::gpu::CanvasBackend {
        self.display.backend()
    }

    /// キャンバスの GPU の表示のテクスチャを読み戻す（試験・計測用。乗算済みの RGBA8、行は文書の下から上）。
    pub fn read_canvas_gpu_display(&mut self, rect: crate::engine::Rect) -> Result<Vec<u8>, String> {
        self.display.read_gpu_display(rect)
    }

    /// キャンバスの GPU の常駐の予算（試験・計測用。既定は `canvas::gpu::RESIDENT_BUDGET`）。
    pub fn set_canvas_gpu_budget(&mut self, bytes: u64) {
        self.display.set_gpu_budget(bytes);
    }

    /// 最後に描いた 3D ビューの中身の表示域（画面の点。隠れていれば None）。
    pub fn view3d_rect(&self) -> Option<Rect> {
        self.view3d.content_rect()
    }

    /// 3D ビューの描画の数（上げたタイル・描いた回数。wgpu が無ければ None）。
    pub fn view3d_stats(&self) -> Option<View3dStats> {
        self.renderer3d.as_ref().map(|r| r.stats)
    }

    /// 3D の GPU の積みが終わるまで待つ・GPU の名前（計測用）。
    pub fn view3d_wait_gpu(&self) {
        if let Some(r) = &self.renderer3d {
            r.wait_gpu();
        }
    }

    pub fn view3d_adapter(&self) -> Option<String> {
        self.renderer3d.as_ref().map(|r| r.adapter_name())
    }

    /// 3D が次のフレームも求めているか（接線を作っている最中など。窓の描き直しの要求と同じ）。
    pub fn view3d_wants_repaint(&self) -> bool {
        self.renderer3d.as_ref().is_some_and(|r| r.wants_repaint())
    }

    /// 試験用: 塗った絵のバイトの予算を小さくして、縮めの道を通す。
    pub fn view3d_set_paint_budget(&mut self, bytes: u64) {
        if let Some(r) = &mut self.renderer3d {
            r.set_paint_budget(bytes);
        }
    }

    /// 試験用: 塗った絵を捨てる（次の描きが文書から全部を作り直す）。
    pub fn view3d_invalidate_paint(&mut self) {
        if let Some(r) = &mut self.renderer3d {
            r.invalidate_paint();
        }
    }

    /// 試験用: 塗った絵のチャンネルの 1 段の中身（形式のバイト列、行は下から。大きさつき）。
    pub fn view3d_read_paint_level(
        &self,
        slot: crate::view3d::paint::Slot,
        level: u32,
    ) -> Option<(Vec<u8>, [u32; 2])> {
        self.renderer3d.as_ref()?.read_paint_level(slot, level)
    }

    /// 試験用: 接線を作るスレッドが仕事の前に呼ぶ口（接線が着く前のフレームを決定的に作る）。
    pub fn view3d_set_tangent_hook(&mut self, hook: Option<crate::view3d::render::TangentHook>) {
        if let Some(r) = &mut self.renderer3d {
            r.set_tangent_hook(hook);
        }
    }

    /// Live Link と同じ形のモデルを読む（Live Link が受けたときと同じ道: 記録・テクスチャセットの結び付け・3D の形。描いている
    /// 最中なら、3D の形は終わってから入れ替わる）。つながりの外から読んだものなので Unity には出さない。
    pub fn load_live_link_model(&mut self, model: &yolu_protocol::Model) -> Result<(), String> {
        self.state.receive_link_model(model, 0).1.map_err(|e| e.to_string())
    }

    /// Live Link と同じ形のポーズを当てる（描いている最中なら、終わってから）。
    pub fn apply_live_link_pose(&mut self, pose: &yolu_protocol::Pose) -> Result<(), String> {
        self.state.receive_link_pose(pose).map_err(|e| e.to_string())
    }

    pub fn pen(&self) -> &PenInput {
        &self.pen
    }

    /// egui が受けるポインタの入力のうち、サイドボタンを押したペンの接触を右ボタンに直す（`eframe::App::raw_input_hook` が毎フレーム
    /// 呼ぶ。winit はペンを左ボタンの押しにしか変えないので、これが無いと、ペンではどの部品の右クリックのメニューも開かない）。
    pub fn remap_pen_buttons(&mut self, events: &mut [egui::Event]) {
        self.pen_buttons.remap(&self.pen.peek(), events);
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
        crate::stencil::update_keys(&ctx, &mut self.state);
        self.open_dropped(&ctx);
        // 一覧の範囲はこのフレームで描いたときだけ入る（棚・チャンネルのタブを開いている間に、前の位置へ落とした PNG を取り込まない）
        self.state.brushes.ui.list_rect = None;
        assets::frame(&ctx, &mut self.state);
        self.handle_requests(&ctx);
        self.link.poll(&mut self.state);
        self.state.link = self.link.view();
        // 別のスレッドの仕事（ベイク・書き出し・PSD）の終わりを受ける
        self.state.poll_bake();
        self.state.poll_export();
        self.state.sync_budgets();
        self.state.poll_psd();
        self.state.poll_brush_import();
        // 効果の入力（焼いたマップ・モデルのルート・画像）を文書へ渡す。入力がそろった読むだけのセットは編集できるようにする
        self.state.sync_effects();
        self.state.poll_newproject();
        // 更新の確かめ・ダウンロードの終わり（準備の窓は、描いている最中は開かない）
        self.state.poll_update();
        self.state.poll_clipboard();
        // 復旧: 書き置きの結果を受け、書く頃なら頼む。フォーカスを失ったら、時間を待たずに書く
        let focused = ctx.input(|i| i.viewport().focused);
        if self.was_focused == Some(true) && focused == Some(false) {
            self.state.recovery_request_flush();
        }
        self.was_focused = focused.or(self.was_focused);
        if let Some(wait) = self.state.recovery_tick() {
            ctx.request_repaint_after(wait);
        }
        // 3D ビューで描くマテリアル・隠すマテリアルを今のテクスチャセットに合わせる（ストロークが終わった後のフレームでも）
        self.state.sync_view3d();
        // ポーズ: 読み終わった FBX を入れる（入れたら 3D ビューのタブを前へ）
        if crate::view3d::pose::frame(&mut self.state, &ctx) {
            if let Some(path) = self.dock.find_tab(&Tab::View3d) {
                let _ = self.dock.set_active_tab(path);
            }
        }
        if self.state.reset_layout {
            self.dock = default_dock();
            self.state.reset_layout = false;
        }

        let mut bar = None;
        let mut link_icon = None;
        egui::Panel::top("yolu.menubar")
            .exact_size(t::MENU_BAR_HEIGHT)
            .frame(Frame::NONE)
            .show(ui, |ui| {
                let r = ui.max_rect();
                let open_menu = match self.state.popup.as_ref().map(|p| p.kind) {
                    Some(PopupKind::MenuBar(i)) => Some(i),
                    _ => None,
                };
                bar = Some(menu::menu_bar_marked(
                    ui,
                    r,
                    &shell::menu_titles(self.state.lang),
                    open_menu,
                    // 新しい版があるあいだ、ヘルプの見出しに印を付ける
                    self.state
                        .update
                        .offer()
                        .map(|_| shell::HELP_MENU),
                ));
                // 右端: プロジェクトの名前と保存の状態。その左に Live Link の入口（Unity の印）。名前は、メニューの見出しの右から窓の右の縁までの
                // 幅（最大 352）に収まるように後ろを詰め、印は「見えている名前」の左に置く（長い名前でも、印がメニューの見出しに重ならない）
                let menu_end = bar
                    .as_ref()
                    .and_then(|b| b.rects.last())
                    .map_or(r.left() + 6.0, |last| last.right());
                let room = (r.right() - 8.0 - (menu_end + 6.0 + shell::LINK_ICON_SLOT + 28.0)).clamp(0.0, 352.0);
                let style = t::LABEL_DIM.with_color(if self.state.modified {
                    t::TEXT
                } else {
                    t::TEXT_DIM
                });
                // 保存していない印（•）の分は、変わっても印が動かないよう、いつも幅に入れる
                let marker_width = w::text_width(ui.painter(), " •", style);
                let shown = w::fit(
                    ui.painter(),
                    &self.state.project_name,
                    (room - marker_width).max(0.0),
                    style,
                );
                let name_width = w::text_width(ui.painter(), &format!("{shown} •"), style);
                let title = Rect::from_min_max(
                    pos2(r.right() - 8.0 - name_width, r.top()),
                    pos2(r.right() - 8.0, r.bottom()),
                );
                let name = format!("{shown}{}", if self.state.modified { " •" } else { "" });
                w::text(ui.painter(), title, &name, style, w::Align::Right);
                if shown != self.state.project_name {
                    // 詰めたときだけ、全体の名前をツールチップに
                    ui.interact(title, ui.id().with("menubar.title"), Sense::hover())
                        .on_hover_text(&self.state.project_name);
                }
                let link_open = matches!(
                    self.state.popup.as_ref().map(|p| p.kind),
                    Some(PopupKind::LiveLink)
                );
                let crash_rect = Rect::from_min_size(
                    pos2(title.left() - shell::LINK_ICON_SLOT - 28.0, r.top()),
                    vec2(26.0, r.height()),
                );
                let mut crash_ui = ui.new_child(egui::UiBuilder::new().max_rect(crash_rect));
                self.state.crash.indicator(&mut crash_ui, self.state.lang);
                link_icon = Some(shell::link_icon(
                    ui,
                    r,
                    r.right() - 8.0 - name_width,
                    &self.state,
                    link_open,
                ));
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
        egui::Panel::bottom("yolu.status")
            .exact_size(t::STATUS_BAR_HEIGHT)
            .frame(Frame::NONE)
            .show(ui, |ui| {
                let r = ui.max_rect();
                shell::status_bar(ui, &self.state, r);
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
                    grabbed: false,
                };
                area(&mut self.dock)
                    .id(Id::new("yolu.dock"))
                    .style(style)
                    .show_close_buttons(false)
                    .show_add_buttons(false)
                    .show_leaf_collapse_buttons(false)
                    .show_leaf_close_all_buttons(false)
                    .show_inside(ui, &mut tabs);
                let grabbed = tabs.grabbed;
                self.tab_rects = tabs.tab_rects;
                self.state.dock_grab = [grabbed, self.state.dock_grab[0]];
            });
        // 3D ビューのタブが見えているか（次のフレームのキー入力・メニューの取り消しの行き先が読む）
        self.state.view3d.visible = self.view3d.content_rect().is_some();
        self.state.canvas_visible = std::mem::take(&mut self.state.canvas_drawn);
        // 隠れたビューは、ペンが離れたのを受け取れない（タブの見出しをつかんで動かしているあいだなど）。ペンの押しの印と、ペンが回し・
        // パン・拡縮していた途中を、見えるようになるまで持ち越さない（印が残ると、ペンの押しとみなしてマウスの押しを使わなくなる）
        if !self.state.canvas_visible {
            self.state.canvas.pen_press = None;
            crate::canvas::nav::cancel(&mut self.state);
        }
        if !self.state.view3d.visible {
            self.state.view3d.input.drop_presses();
        }

        let bar = bar.unwrap_or(menu::BarOutcome {
            rects: Vec::new(),
            pressed: None,
            hovered: None,
        });
        // スライダーのドラッグを押したまま Esc で止めた: スライダーは押し始めの値へ戻して残りのドラッグを受けないので、
        // ここで（全部のスライダーが動いたあとに）まとめていた変更を段ごと捨てる
        if ctx.input(|i| i.key_pressed(egui::Key::Escape) && i.pointer.primary_down()) {
            self.state.m2_cancel_drag();
        }
        self.popups(&ctx, &bar, link_icon);
        crate::selection::dialog::show(&ctx, &mut self.state);
        crate::windows::show(&ctx, &mut self.state);
        crate::prefs::show(&ctx, &mut self.state);
        crate::recovery::window::show(&ctx, &mut self.state);
        self.state.crash.show(&ctx, self.state.lang);
        crate::crash::message(&self.state.message);
        if self.dialogs {
            self.state.crash.execute_request(self.state.lang);
        }
        let popup_rect = self.state.popup.as_ref().map(|p| p.state.rect);
        self.view3d.end_frame(popup_rect);
        // メニューで選んだ Live Link・ファイルの頼みはこのフレームのうちに当て、描いた所を Unity へ出す
        self.handle_requests(&ctx);
        self.link.publish(&mut self.state);
        self.state.link = self.link.view();
        // 「保存して更新」: 保存先を選ぶ窓も済んだこのフレームの終わりに、保存の結果を見て入れる
        self.state.update_finish_save();
        // 終了・窓を閉じる: 保存していない変更があれば聞く（窓を開かない試験では聞かない）
        let close_requested = ctx.input(|i| i.viewport().close_requested());
        if (self.state.quit || close_requested) && !self.closing {
            if self.confirm_close() {
                self.closing = true;
                crate::windows::stop_jobs(&mut self.state, std::time::Duration::from_secs(3));
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

    fn popups(&mut self, ctx: &egui::Context, bar: &menu::BarOutcome, link_icon: Option<shell::LinkIcon>) {
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
        // Live Link の入口: 押したら窓を開く（開いていれば閉じる）
        if let Some(icon) = link_icon.filter(|i| i.pressed) {
            if matches!(self.state.popup.as_ref().map(|p| p.kind), Some(PopupKind::LiveLink)) {
                self.state.popup = None;
            } else if !self.state.is_stroking() {
                self.state.popup = Some(OpenPopup {
                    kind: PopupKind::LiveLink,
                    state: PopupState::new(ctx, icon.rect),
                });
            }
        }
        let Some(mut open) = self.state.popup.take() else {
            return;
        };
        let entries = shell::popup_entries(&self.state, open.kind);
        let keep: Vec<Rect> = match open.kind {
            PopupKind::MenuBar(_) => bar.rects.clone(),
            PopupKind::LiveLink => link_icon.map(|i| vec![i.rect]).unwrap_or_default(),
            _ => Vec::new(),
        };
        match menu::show(ctx, Id::new("yolu.popup"), &mut open.state, &entries, &keep) {
            PopupOutcome::Open => self.state.popup = Some(open),
            PopupOutcome::Close => {}
            PopupOutcome::Chosen(action) => self.state.apply(action),
            PopupOutcome::Step(d) => {
                if let PopupKind::MenuBar(i) = open.kind {
                    let n = shell::MENU_TITLES.len() as i32;
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

/// 設定の表示の合成から、キャンバスの表示の方針。自動は環境変数 `YOLUPAINTER_CANVAS`（無ければ自動）。
fn canvas_backend(setting: crate::settings::Compositing) -> crate::canvas::gpu::CanvasBackend {
    use crate::canvas::gpu::CanvasBackend;
    use crate::settings::Compositing;
    match setting {
        Compositing::Auto => CanvasBackend::from_env(),
        Compositing::Gpu => CanvasBackend::Gpu,
        Compositing::Cpu => CanvasBackend::Cpu,
    }
}

/// 起動の引数（実行ファイルの名前のあと）が .ylp ならそのパス。関連付けとエクスプローラーの「プログラムから開く」が渡す形。
/// 無い・開けないファイルでも渡す（黙って空の画面を出さず、開く処理が理由を知らせる）。.ylp 以外は開かない。
fn startup_project(mut args: impl Iterator<Item = std::ffi::OsString>) -> Option<std::path::PathBuf> {
    let path = std::path::PathBuf::from(args.nth(1)?);
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("ylp"))
        .then_some(path)
}

/// 起動時の状態の帯の知らせ（既定へ戻した設定の理由。あれば全部）。ペンを受けていることは知らせない（状態の文を帯に出さない）。
fn startup_message(lang: crate::lang::Lang, problems: &[Problem]) -> Option<String> {
    let parts: Vec<String> = problems.iter().map(|p| p.text(lang)).collect();
    (!parts.is_empty()).then(|| parts.join(" "))
}

impl eframe::App for YoluApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        self.apply_compositing();
        self.frame(ui);
        self.persist_settings();
    }

    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        self.remap_pen_buttons(&mut raw_input.events);
    }

    /// 正しく終わった: 変更があれば最後の世代を書き、復旧の印を消す（世代は設定の数だけ残す）。
    fn on_exit(&mut self) {
        self.state.recovery_shutdown();
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        egui::Rgba::from(t::WINDOW_BG).to_array()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lang::Lang;

    #[test]
    fn only_an_existing_ylp_argument_opens_at_startup() {
        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/startup-arg-tests")
            .join(std::process::id().to_string());
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let project = dir.join("A.YLP");
        let other = dir.join("a.png");
        std::fs::write(&project, b"x").unwrap();
        std::fs::write(&other, b"x").unwrap();
        let args = |list: &[&std::path::Path]| {
            std::iter::once(std::ffi::OsString::from("yolupainter"))
                .chain(list.iter().map(|p| p.as_os_str().to_owned()))
                .collect::<Vec<_>>()
                .into_iter()
        };
        assert_eq!(startup_project(args(&[&project])), Some(project.clone()));
        assert_eq!(startup_project(args(&[])), None);
        assert_eq!(startup_project(args(&[&other])), None);
        // 無い .ylp も渡す（開く処理が「開けません」を知らせる）
        let missing = dir.join("missing.ylp");
        assert_eq!(startup_project(args(&[&missing])), Some(missing));
        std::fs::remove_dir_all(dir).unwrap();
    }

    fn startup_app(args: &[&std::path::Path]) -> YoluApp {
        let mut app = YoluApp::with_state(crate::state::AppState::new(64, 64), PenInput::detached());
        app.open_startup_project(
            std::iter::once(std::ffi::OsString::from("yolupainter"))
                .chain(args.iter().map(|p| p.as_os_str().to_owned())),
        );
        app
    }

    #[test]
    fn a_ylp_argument_opens_through_the_app_and_a_bad_one_says_why() {
        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/startup-open-tests")
            .join(std::process::id().to_string());
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // 保存した .ylp を、引数で開く
        let project = dir.join("Opened.ylp");
        let mut source = crate::state::AppState::new(64, 64);
        source.apply(Action::SaveProjectAs(project.clone()));
        assert!(!source.modified, "{}", source.message);
        let app = startup_app(&[&project]);
        assert_eq!(app.state.project.as_ref().map(|p| p.path().to_path_buf()), Some(project.clone()));
        assert!(!app.state.message.contains("開けません"), "{}", app.state.message);
        // 壊れた .ylp・無い .ylp は、開かずに理由を知らせる（元の文書はそのまま）
        let broken = dir.join("Broken.ylp");
        std::fs::write(&broken, b"not a project").unwrap();
        for bad in [broken, dir.join("missing.ylp")] {
            let app = startup_app(&[&bad]);
            assert!(app.state.project.is_none(), "{}", bad.display());
            assert!(
                app.state.message.starts_with("開けません: "),
                "{}: {}",
                bad.display(),
                app.state.message
            );
        }
        // .ylp ではない引数・引数なしは、何も開かず、知らせもない
        let other = dir.join("a.png");
        std::fs::write(&other, b"x").unwrap();
        for args in [vec![other.as_path()], Vec::new()] {
            let app = startup_app(&args);
            assert!(app.state.project.is_none() && app.state.message.is_empty());
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn startup_message_keeps_all_notices() {
        assert_eq!(startup_message(Lang::Ja, &[]), None);
        assert_eq!(startup_message(Lang::En, &[Problem::Unreadable]).as_deref(), Some("Cannot read the settings."));
        assert_eq!(
            startup_message(Lang::En, &[Problem::Language("x".into())]).as_deref(),
            Some("Cannot read the language setting.")
        );
        // 読めなかった設定の理由が 2 つ以上なら全部
        let both = startup_message(
            Lang::Ja,
            &[Problem::Language("x".into()), Problem::Invalid { key: "cpu_threads", value: "0".into() }, Problem::Backups("-2".into())],
        )
        .unwrap();
        assert!(
            both.contains("言語の設定を読めません") && both.contains("CPU のスレッド") && both.contains("退避を残す数") && !both.contains("Windows Ink"),
            "{both}"
        );
    }
}
