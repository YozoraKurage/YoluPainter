//! アプリの本体: 外枠（メニューバー・オプションバー・ツールの帯・ステータスバー）と、egui_dock のドッキング（Substance の並び。
//! 左: アセットとカラー、中央: キャンバスと 3D ビュー、右: テクスチャセットとレイヤーとプロパティ）、ポップアップ、3D ビューの知らせ、
//! Live Link（フレームの頭で受け、終わりに変わったタイルを出す）、ファイルの窓。

use std::collections::HashMap;

use egui::{pos2, vec2, Color32, Frame, Id, Rect, RichText, Sense, Ui, WidgetText};
use egui_dock::{DockArea, DockState, NodeIndex, TabViewer};

use crate::canvas::{self, display::CanvasDisplay};
use crate::livelink::LiveLink;
use crate::mcp_server::McpServer;
use crate::panels::{
    assets, color::ColorTextures, layers, layers::Thumbnails, properties, texture_sets,
    view3d::View3dHost, view3d::View3dSlot,
};
use crate::pen::{PenInput, PenSample};
use crate::settings::{Problem, Settings};
use crate::shell;
use crate::state::{Action, AppState, DialogRequest, OpenPopup, PopupKind, DEFAULT_DOCUMENT_SIZE};
use crate::titlebar;
use crate::ui::fonts;
use crate::ui::menu::{self, PopupOutcome, PopupState};
use crate::ui::theme as t;
use crate::ui::{icons, widgets as w};
use crate::view3d::render::{View3dRenderer, View3dStats};

/// ドックのタブ。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tab {
    /// サブツールの一覧・ツールプロパティ・ブラシサイズ（左のドックの先頭。中身は今の道具に合わせて替わる）。
    SubTools,
    Assets,
    Color,
    /// ポーズ（ボーンのインスペクター・BlendShape・面を隠す）。スキンのあるモデルを読むと、プロパティと同じ組へ足される。
    Pose,
    Canvas,
    View3d,
    TextureSets,
    Layers,
    Properties,
    Channels,
    History,
    ColorSets,
    Navigator,
    /// 起動してからの注意と失敗の知らせ（既定の並びではレイヤーと同じ組の後ろ。プロパティ・ヒストリーの組に 3 つ並べると、いちばん小さい窓で
    /// 英語の名前が欠ける。古い並びに無ければ、読んだときにレイヤーの組へ足す）。
    Log,
}

impl Tab {
    /// ドックに出るタブ全部（ポーズは、スキンのあるモデルを読むと足される）。
    pub const ALL: [Tab; 14] = [
        Tab::SubTools,
        Tab::Assets,
        Tab::Color,
        Tab::Pose,
        Tab::Canvas,
        Tab::View3d,
        Tab::TextureSets,
        Tab::Layers,
        Tab::Properties,
        Tab::Channels,
        Tab::History,
        Tab::ColorSets,
        Tab::Navigator,
        Tab::Log,
    ];

    /// 保存する名前（並びのファイル `layout.json` に書く。Rust の名前を変えても変わらないよう、ここで決める。足すのは良いが、
    /// 書き換えると保存済みの並びが「知らないタブ」で捨てられる）。
    pub fn key(self) -> &'static str {
        match self {
            Tab::SubTools => "subtools",
            Tab::Assets => "assets",
            Tab::Color => "color",
            Tab::Pose => "pose",
            Tab::Canvas => "canvas",
            Tab::View3d => "view3d",
            Tab::TextureSets => "texture_sets",
            Tab::Layers => "layers",
            Tab::Properties => "properties",
            Tab::Channels => "channels",
            Tab::History => "history",
            Tab::ColorSets => "color_sets",
            Tab::Navigator => "navigator",
            Tab::Log => "log",
        }
    }

    /// 保存した名前から。知らない名前は None。
    pub fn from_key(key: &str) -> Option<Tab> {
        Tab::ALL.into_iter().find(|t| t.key() == key)
    }

    pub fn title(self) -> &'static str {
        self.title_in(crate::lang::Lang::Ja)
    }

    /// 言語ごとのタブの名前。
    pub fn title_in(self, lang: crate::lang::Lang) -> &'static str {
        match self {
            Tab::SubTools => lang.pick("サブツール", "Tools"),
            Tab::Assets => lang.pick("アセット", "Assets"),
            Tab::Color => lang.pick("カラー", "Color"),
            Tab::Pose => lang.pick("ポーズ", "Pose"),
            Tab::Canvas => lang.pick("キャンバス", "Canvas"),
            Tab::View3d => lang.pick("3D ビュー", "3D View"),
            Tab::TextureSets => lang.pick("テクスチャセット", "Texture Sets"),
            Tab::Layers => lang.pick("レイヤー", "Layers"),
            Tab::Properties => lang.pick("プロパティ", "Properties"),
            Tab::Navigator => lang.pick("ナビゲーター", "Navigator"),
            Tab::Channels => lang.pick("チャンネル", "Channels"),
            Tab::History => lang.pick("ヒストリー", "History"),
            Tab::ColorSets => lang.pick("カラーセット", "Color Sets"),
            Tab::Log => lang.pick("ログ", "Log"),
        }
    }
}

impl serde::Serialize for Tab {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.key())
    }
}

impl<'de> serde::Deserialize<'de> for Tab {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Tab, D::Error> {
        use serde::de::Error;
        let key = String::deserialize(deserializer)?;
        Tab::from_key(&key).ok_or_else(|| D::Error::custom(format!("知らないタブ「{key}」")))
    }
}

/// Substance Painter の並び（左: サブツールとアセットとチャンネルとカラー、中央: キャンバスと 3D ビュー、右: 上からテクスチャセット・レイヤー・
/// プロパティ）。サブツールのパネルは一覧・ツールプロパティ・ブラシサイズが縦に入るので、左の列の上を高めに取る。
/// 左の列は、いちばん小さい窓（960 点）でも 3 つのタブ（英語の Tools・Assets・Channels）の見出しが収まる割合（1600 点の窓で約 330 点）。
/// 右の列は 1600 点の幅の窓で約 300 点（左の列を広げた分、中の割合を減らして右の幅を前と同じにした）。egui_dock の割合は左（上）の子の取り分（分けた向きによらない）。
pub fn default_dock() -> DockState<Tab> {
    let mut dock = DockState::new(vec![Tab::Canvas, Tab::View3d]);
    let surface = dock.main_surface_mut();
    let [center, left] = surface.split_left(
        NodeIndex::root(),
        0.21,
        vec![Tab::SubTools, Tab::Assets, Tab::Channels],
    );
    // ナビゲーターはテクスチャセットと同じ組（左下は狭く、カラー・カラーセットと 3 つ並べると最小の窓で名前が欠ける）
    let [_, right] = surface.split_right(center, 0.764, vec![Tab::TextureSets, Tab::Navigator]);
    surface.split_below(left, 0.66, vec![Tab::Color, Tab::ColorSets]);
    let [_, layers] = surface.split_below(right, 0.24, vec![Tab::Layers, Tab::Log]);
    surface.split_below(layers, 0.45, vec![Tab::Properties, Tab::History]);
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
            Tab::Navigator => crate::navigator::show(ui, self.app),
            Tab::Canvas => canvas::show(ui, self.app, self.display, self.pen),
            Tab::View3d => self
                .view3d
                .show(ui, self.app, self.renderer3d.as_mut(), self.pen),
            Tab::TextureSets => texture_sets::show(ui, self.app),
            Tab::Channels => crate::panels::channels::show(ui, self.app),
            Tab::Layers => layers::show(ui, self.app, self.thumbs),
            Tab::Color => crate::panels::color::show(ui, self.app, self.colors),
            Tab::Pose => crate::panels::pose::show(ui, self.app),
            Tab::Properties => properties::show(ui, self.app),
            Tab::History => crate::panels::history::show(ui, self.app),
            Tab::Assets => assets::show(ui, self.app),
            Tab::SubTools => crate::panels::subtools::show(ui, self.app),
            Tab::ColorSets => crate::panels::colorsets::show(ui, self.app),
            Tab::Log => crate::panels::log::show(ui, self.app),
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

/// 終わってよいかの問いへの答え（試験が窓を開かずに答える口）。
type CloseAnswer = Box<dyn FnMut(&AppState) -> bool>;

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
    /// 主の wgpu の装置の見張り（実際の窓だけ。`watch_gpu`）。装置を失ったときの流れは `gpu_lost`。
    gpu_watch: Option<crate::gpu_watch::GpuWatch>,
    /// 主の wgpu の装置を失った（このあと終わる）。
    gpu_lost: Option<crate::gpu_watch::Lost>,
    /// 装置を失ったとき、書き置きの書き込みを待つ長さの上限（取るときも、終わるときも。試験が短くする）。
    gpu_lost_wait: std::time::Duration,
    /// 最後のフレームのドックのタブのボタンの矩形（試験用。ドックのタブは読み上げの名前を持たない）。
    pub tab_rects: HashMap<Tab, Rect>,
    link: LiveLink,
    /// 外からの操作（MCP のクライアント・コマンドライン）を受ける（設定「外からの操作を受ける」が入っている間だけ 127.0.0.1 で待つ）。
    ops: McpServer,
    /// ファイルの窓・確かめの窓を開くか（eframe の窓だけ。試験では開かず、頼みを `state.dialog_request` に残す）。
    dialogs: bool,
    /// 終わると決めた（閉じる頼みを二度聞かない）。
    closing: bool,
    /// OS の終了を待たせる印（`session_end::set_saving`）に最後に伝えた「保存の間か」。窓が見えている間は `ui`、隠れている間は `logic` が
    /// 保存の結果を受けるたびに合わせる。
    saving_marked: bool,
    /// 試験用: 保存していない変更のまま終わってよいかの問いに、窓を開かずに答える（窓を開かない試験が、保存の後に問われるかを見る）。
    close_answer: Option<CloseAnswer>,
    /// OS の枠を外した窓か（Windows の実際の窓だけ true。帯の右端に最小化・最大化・閉じるを置き、窓の縁で大きさを変える）。
    /// 設定には出さない。試験は `set_custom_frame` で選ぶ。
    custom_frame: bool,
    /// 前のフレームの帯で、egui の押しを持たずに生の押しで動く部品（Live Link の印）の矩形。窓の縁は、この上の押しを譲る
    /// （egui の当たり判定には出ないので、縁の側へ矩形で渡す）。
    bar_press_rects: Vec<Rect>,
    settings: Option<(std::path::PathBuf, Settings)>,
    /// 前のフレームでウィンドウにフォーカスがあったか（失ったら復旧の書き置きを待たずに書く）。
    was_focused: Option<bool>,
    /// 表示の合成の設定で、キャンバスの表示に入れた値（変わったときだけ入れ直す。試験や環境変数で決めた方針を、設定が変わらないうちは
    /// 上書きしない）。
    compositing_applied: crate::settings::Compositing,
    /// GPU のメモリの設定から配った予算で、3D の絵・キャンバスの合成・棚へ入れた値（変わったときだけ入れ直す。試験が決めた予算を、
    /// 設定とアダプターが変わらないうちは上書きしない）。
    gpu_budgets_applied: crate::gpu_memory::Budgets,
    /// GPU の確保済みのメモリを測る wgpu の装置（状態の帯の右端のメモリ。装置が無い試験は None）。
    gpu_device: Option<eframe::egui_wgpu::wgpu::Device>,
    /// ドックの並びと窓の大きさ・位置を書く場所（設定のフォルダの `layout.json`。設定のフォルダが無い試験は None）。
    layout_path: Option<std::path::PathBuf>,
    /// 最後に書いた（または読んだ）ファイルの中身。変わったときだけ書く。
    layout_saved: String,
    /// 最後に「書くか」を見た時刻（egui の時刻。1 秒おきに見る）。
    layout_checked_at: f64,
    /// 最後に見た、最大化していない窓の大きさと位置（終わるときに書く）。
    window_record: Option<crate::layout::WindowRecord>,
    /// 起動のあと、窓が画面より大きくないか確かめたか。
    window_checked: bool,
    /// 起動のあと、窓が画面より大きければ収めるか（実際の窓だけ。試験は `fit_to_screen` で選ぶ）。
    fit_window: bool,
    /// 浮かせた窓の今の位置と大きさ（保存に入れる。egui_dock は窓の矩形を自分では更新しない）。
    float_rects: Vec<crate::layout::FloatRect>,
    /// 落とした PSD のうち、取り込まなかった数（取り込みの仕事が終わったときの文に、理由として足す。0 なら無い）。
    psd_drop_more: usize,
}

mod gpu_lost;

impl YoluApp {
    /// 文脈に配色・書体・アイコンを入れる（窓を作るときに 1 度）。
    pub fn setup(ctx: &egui::Context) {
        // egui の既定は、Ctrl+-・Ctrl++・Ctrl+0 で画面全体（文字も部品も）の拡大率を変える。アプリのキーはこの組み合わせを
        // キャンバスの拡大・縮小に使うので、同じキーで画面全体まで縮んで戻せなくなる。画面全体の拡大縮小は切る
        ctx.options_mut(|o| o.zoom_with_keyboard = false);
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
        // OS の終了が保存の途中に来たら、保存が終わるまで待ってもらう（Windows だけ）
        crate::session_end::attach(cc);
        let mut app =
            YoluApp::with_settings(crate::settings::path(), pen, crate::lang::system_lang())
                .with_render_state(cc.wgpu_render_state.as_ref());
        // 主の装置を失ったとき・受け手の無い誤りを受ける（wgpu の既定は、失っても黙り、誤りは panic で落とす）
        if let Some(rs) = &cc.wgpu_render_state {
            app.watch_gpu(rs, &cc.egui_ctx);
        }
        app.dialogs = true;
        // 保存は裏のスレッドで動かす（描ける・見られる。試験の状態は、保存の頼みの中で終える）
        app.state.save.background = true;
        app.fit_window = true;
        // Windows は OS の枠を外している（main.rs）ので、帯と縁は自前
        app.custom_frame = titlebar::CUSTOM_FRAME;
        // 状態の帯の右端に版とビルドを出す（実際の窓だけ。試験の画像がコミットごとに変わらないように）
        app.state.usage.build = Some(crate::usage::build_label());
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
        app.start_live_link(&cc.egui_ctx, std::env::args_os());
        // 外からの操作を受ける設定が入っていれば、起動のうちに待ち受ける
        app.tick_ops(&cc.egui_ctx);
        app
    }

    /// 実アプリ起動時の受け付け（設定「Unity の Live Link を受け付ける」、または `--livelink`）。試験はフォルダと引数を差し替えて同じ経路を通す。
    pub fn start_live_link(
        &mut self,
        ctx: &egui::Context,
        args: impl Iterator<Item = std::ffi::OsString>,
    ) {
        self.link.arm(ctx);
        if args.skip(1).any(|arg| arg == "--livelink") {
            self.link.force();
        }
        // 起動時の知らせを受け付けの文で上書きしない。状態は右端の印とツールチップに出す。
        let link = &mut self.link;
        self.state.keep_notice(|state| link.poll(state));
        self.state.link = self.link.view(&self.state);
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
        // 起動時の知らせ（あれば）に添える: 復旧を使えない・設定を読めない（気をつけること）
        if let Some(note) = note {
            self.state.amend(
                crate::notice::Kind::Warning,
                crate::notice::Source::Recovery,
                " ",
                &note,
            );
        }
    }

    /// 起動の引数（実行ファイルの名前のあと）に .ylp があれば開く。開けないときは、`OpenProject` が message に理由を書く。
    pub fn open_startup_project(&mut self, args: impl Iterator<Item = std::ffi::OsString>) {
        if let Some(path) = startup_project(args) {
            self.state.apply(Action::OpenProject(path));
        }
    }

    /// 設定のファイル（無ければ保存しない）から言語・書き出しの余白・メモリの予算・退避を残す数などを決めて作る（`setup` は呼ぶ側で）。
    /// 最初のレイヤー・テクスチャセット・プロジェクトの名前がその言語になる。読めない設定・正しくない値は既定（自動の予算・
    /// すべて残す、など）に戻し、理由を知らせる
    /// （ファイルは、設定を選び直すまで触らない）。言語は、設定に書いてあればそれ、無い・読めないときだけ `system`（OS の言語）。
    fn with_settings(
        settings: Option<std::path::PathBuf>,
        pen: PenInput,
        system: crate::lang::Lang,
    ) -> YoluApp {
        let (loaded, problems) = match settings.as_deref() {
            Some(path) => crate::settings::load_for_startup(path, system),
            None => (
                Settings {
                    lang: system,
                    ..Default::default()
                },
                Vec::new(),
            ),
        };
        let lang = loaded.lang;
        let mut app = YoluApp::with_state(
            AppState::new_in(DEFAULT_DOCUMENT_SIZE, DEFAULT_DOCUMENT_SIZE, lang),
            pen,
        );
        app.state.load_settings(loaded.clone());
        // 前のプロセスが残したディスクキャッシュのファイル（電源が落ちたときなど）を、起動の邪魔をしないよう裏で消す
        let cache_folder = loaded.disk_cache_folder();
        let _ = std::thread::Builder::new()
            .name("yolu-cache-sweep".into())
            .spawn(move || yolu_core::tile_cache::remove_stale_files(&cache_folder));
        // 利用者のブラシは設定のフォルダの brushes/（読めないファイルは読み飛ばし、知らせる）
        if let Some(dir) = settings.as_deref().and_then(|p| p.parent()) {
            app.state.attach_brush_store(dir.join("brushes"));
            app.state.attach_subtool_store(dir.join("subtools"));
            app.state.ramp_sets.attach(dir.join("gradients"));
            app.state
                .view3d
                .pose
                .hide_presets
                .attach(dir.join("hide_presets"));
            app.state
                .view3d
                .pose
                .pose_presets
                .attach(dir.join("pose_presets"));
        }
        // サムネイルは中身の札でキャッシュのフォルダに覚える（作り直せる写し。設定のファイルが無ければ覚えない）
        app.state
            .library
            .attach_cache(settings.as_deref().and_then(crate::library::cache::dir_for));
        let mut notices: Vec<String> = Vec::new();
        notices.extend(startup_message(lang, &problems));
        notices.extend(app.state.brush_problem_message());
        notices.extend(app.state.subtool_problem_message());
        notices.extend(app.state.ramp_sets.problem().map(|e| {
            lang.pick(
                format!("グラデーションセットを読めません。{}", e.describe(lang)),
                format!("Cannot read the gradient sets. {}", e.describe(lang)),
            )
        }));
        // カラーセット（読めなかった物は、起動時の知らせに「 / 」で添える）
        let colorsets = settings
            .as_deref()
            .and_then(|p| p.parent())
            .and_then(|dir| crate::colorsets::attach(&mut app.state, dir.join("colorsets")));
        // 起動時に読めなかった設定・ブラシ・サブツール・グラデーションセット・カラーセット（既定で始めた）: 気をつけること
        let mut startup = notices.join(" ");
        if let Some(colorsets) = colorsets {
            if !startup.is_empty() {
                startup.push_str(" / ");
            }
            startup.push_str(&colorsets);
        }
        if !startup.is_empty() {
            app.state.warn(crate::notice::Source::Settings, startup);
        }
        // 「起動時に更新を確かめる」の選択は、言語の設定と同じフォルダの別のファイル
        if let Some(path) = settings
            .as_deref()
            .and_then(crate::update::config::path_for)
        {
            app.state.update.attach_config(path);
        }
        // 表示の合成の設定（自動のときは、環境変数 `YOLUPAINTER_CANVAS` か自動のまま）
        app.compositing_applied = loaded.compositing;
        if loaded.compositing != crate::settings::Compositing::Auto {
            app.display.set_backend(canvas_backend(loaded.compositing));
        }

        // ドックの並びと窓の大きさ・位置は、設定のフォルダの layout.json から戻す（読めない・古い・知らないタブは捨てて既定の並び。
        // 理由は診断のログだけ）
        if let Some(path) = settings.as_deref().and_then(crate::layout::path_for) {
            let layout = crate::layout::load(&path);
            for reason in &layout.problems {
                crate::crash::problem(reason.clone());
            }
            if let Some(dock) = layout.dock {
                app.dock = dock;
            }
            app.window_record = layout.window;
            if path.exists() && layout.problems.is_empty() {
                app.layout_saved = crate::layout::render(&app.dock, app.window_record.as_ref());
            }
            app.layout_path = Some(path);
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

    /// GPU のメモリの設定（と、アダプターから分かった量）が配る予算が変わっていれば、3D の絵・キャンバスの合成・棚へ入れる（次のフレームから効く）。
    /// スライダーをドラッグしている間は入れず、離したフレームで入れる（キャンバスの合成は、入れ直すたびに GPU の資源を手放す）。
    fn apply_gpu_memory(&mut self) {
        if self.state.prefs.dragging {
            return;
        }
        let budgets = self.state.gpu_budgets();
        if budgets == self.gpu_budgets_applied {
            return;
        }
        self.gpu_budgets_applied = budgets;
        if let Some(r) = &mut self.renderer3d {
            r.set_paint_budget(budgets.paint);
        }
        self.display.set_gpu_budget(budgets.canvas);
        self.state.shelf.set_preview_budget(budgets.shelf_preview);
    }

    /// 文脈と設定のファイルから作る（試験用。`for_context` に、設定の読み書きを足したもの。設定に言語が無いときは既定の日本語）。
    pub fn for_context_with_settings(
        ctx: &egui::Context,
        settings: Option<std::path::PathBuf>,
        pen: PenInput,
    ) -> YoluApp {
        Self::for_context_with_system_lang(ctx, settings, pen, crate::lang::Lang::default())
    }

    /// `for_context_with_settings` の、OS の言語（設定に言語が無いときの言語）を渡せるもの（試験用。本物は `lang::system_lang`）。
    pub fn for_context_with_system_lang(
        ctx: &egui::Context,
        settings: Option<std::path::PathBuf>,
        pen: PenInput,
        system: crate::lang::Lang,
    ) -> YoluApp {
        Self::setup(ctx);
        YoluApp::with_settings(settings, pen, system)
    }

    /// 設定（言語・書き出しの余白・メモリの予算・スレッド・合成・棚の場所・退避を残す数・選択範囲の帯）の選択が変わっていれば、設定のファイルに書く。
    /// 書けなくても動作は変えず、知らせるだけ。失敗しても同じ選択では再試行しない（毎フレームの I/O と、知らせの上書きを避ける）。
    /// 退避の数・UV ワイヤーフレームの色・筆圧の調整・3D の仕上げ・3D の塗りの切り替えは、スライダーをドラッグしている間は書かない（離したとき、または Esc で戻した値が書いてある値と同じなら書かない）。
    fn persist_settings(&mut self) {
        crate::colorsets::persist(&mut self.state);
        let Some((path, saved)) = &mut self.settings else {
            return;
        };
        let mut now = self.state.settings();
        if self.state.prefs.dragging {
            now.backups = saved.backups;
            now.uv_wireframe_color = saved.uv_wireframe_color;
            now.gpu_memory = saved.gpu_memory;
        }
        if self.state.pressure.dragging {
            now.pressure = saved.pressure.clone();
        }
        // 3D の仕上げのスライダー（ブルーム）と 3D の塗りの切り替えのスライダーも、ドラッグ中は書かず、離したときの値を書く
        if self.state.view3d.display.post_dragging {
            now.view3d_post = saved.view3d_post;
        }
        if self.state.view3d.projection_dragging {
            now.view3d_paint = saved.view3d_paint;
        }
        if *saved == now {
            return;
        }
        *saved = now.clone();
        if crate::settings::save(path, &now).is_err() {
            self.state.fail(
                crate::notice::Source::Settings,
                now.lang
                    .pick("設定を保存できません。", "Cannot save the settings."),
            );
        }
    }

    /// ドックの並びと窓の大きさ・位置を、変わっていれば `layout.json` へ書く（毎フレーム呼び、1 秒おきに見る。区切りを動かしている間も
    /// 1 秒おきに 1 回までなので、書き込みが続かない。タブの見出しをつかんでいる間は並びが変わらない）。窓の大きさと位置は、最大化していない
    /// 間の値を覚える（最大化したまま終わっても、戻したときの大きさを書く）。書けなくても動作は変えない（診断のログへ。同じ中身では書き直さない）。
    fn persist_layout(&mut self, ctx: &egui::Context) {
        // 起動の窓の置き場所を合わせている間（最初の数フレーム）は、途中の位置を記録・保存しない
        if crate::windowpos::settle(ctx) {
            return;
        }
        let info = ctx.input(|i| i.viewport().clone());
        let maximized = info.maximized.unwrap_or(false);
        if !maximized && !info.fullscreen.unwrap_or(false) && !info.minimized.unwrap_or(false) {
            if let (Some(outer), Some(inner)) = (info.outer_rect, info.inner_rect) {
                self.window_record = Some(crate::layout::WindowRecord {
                    position: [outer.min.x, outer.min.y],
                    size: [inner.width(), inner.height()],
                    pixels_per_point: info.native_pixels_per_point.unwrap_or(1.0),
                    maximized: false,
                });
            }
        }
        if let Some(record) = &mut self.window_record {
            record.maximized = maximized;
        }
        // 起動のあと 1 度、窓が画面より大きければ画面に収める（保存したあとで画面が小さくなったとき）
        if self.fit_window && !self.window_checked {
            if let (Some(monitor), Some(inner)) = (info.monitor_size, info.inner_rect) {
                self.window_checked = true;
                if !maximized && (inner.width() > monitor.x || inner.height() > monitor.y) {
                    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(vec2(
                        inner.width().min(monitor.x),
                        inner.height().min(monitor.y),
                    )));
                }
            }
        }
        self.remember_floats(ctx);
        let now = ctx.input(|i| i.time);
        if now - self.layout_checked_at < 1.0 {
            return;
        }
        self.layout_checked_at = now;
        self.save_layout(false);
    }

    /// 浮かせた窓の今の位置と大きさを、egui が覚えている窓の矩形から控える（egui_dock が窓に命じる大きさは枠を含む外側の大きさなので、
    /// 矩形をそのまま読み戻しに使える）。まだ描いていない窓は控えない（読んだ値か、最初に描くときの値がそのまま残る）。
    fn remember_floats(&mut self, ctx: &egui::Context) {
        self.float_rects.clear();
        for (index, surface) in self.dock.iter_surfaces_indexed() {
            if !matches!(surface, egui_dock::Surface::Window(..)) {
                continue;
            }
            // egui_dock が窓に付ける名前（`window {面の番号}`）
            let id = Id::new(format!("window {index:?}"));
            if let Some(rect) = ctx.memory(|m| m.area_rect(id)) {
                self.float_rects.push((index, rect));
            }
        }
    }

    /// 並びを書く。`force` でなければ、前に書いた中身と同じなら書かない。
    fn save_layout(&mut self, force: bool) {
        let Some(path) = &self.layout_path else {
            return;
        };
        let window = self
            .window_record
            .and_then(crate::layout::WindowRecord::sanitized);
        let text = crate::layout::render_with(&self.dock, window.as_ref(), &self.float_rects);
        if !force && text == self.layout_saved {
            return;
        }
        // 書けなくても、同じ中身では書き直さない（毎秒の入出力と、診断のログの繰り返しを避ける）
        self.layout_saved = text.clone();
        if let Err(e) = crate::layout::save(path, &text) {
            crate::crash::problem(format!("画面の並びを保存できません（{e}）。"));
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
            gpu_watch: None,
            gpu_lost: None,
            gpu_lost_wait: gpu_lost::RECOVERY_WAIT,
            tab_rects: HashMap::new(),
            link: LiveLink::new(),
            ops: McpServer::new(),
            dialogs: false,
            closing: false,
            close_answer: None,
            custom_frame: false,
            bar_press_rects: Vec::new(),
            settings: None,
            was_focused: None,
            psd_drop_more: 0,
            compositing_applied: crate::settings::Compositing::Auto,
            gpu_budgets_applied: crate::gpu_memory::Budgets::default(),
            gpu_device: None,
            layout_path: None,
            layout_saved: String::new(),
            layout_checked_at: f64::NEG_INFINITY,
            window_record: None,
            window_checked: false,
            fit_window: false,
            float_rects: Vec::new(),
            saving_marked: false,
        }
    }

    /// 起動のあと 1 度、窓が画面より大きければ画面に収める（実際の窓だけ。保存したあとで画面が小さくなったときのため）。
    pub fn fit_to_screen(mut self, on: bool) -> YoluApp {
        self.fit_window = on;
        self
    }

    /// 試験用: 保存していない変更のまま終わってよいかの問いに、窓を開かずに答える（問われるたびに呼ぶ。保存の後の状態で問われること・
    /// 問われないことを確かめる）。
    #[doc(hidden)]
    pub fn answer_close_question(&mut self, answer: impl FnMut(&AppState) -> bool + 'static) {
        self.close_answer = Some(Box::new(answer));
    }

    /// 終わると決めたか（保存の途中なら、保存が終わるまで決めない）。
    pub fn is_closing(&self) -> bool {
        self.closing
    }

    /// 試験用: OS の終了を待たせる印に、最後に「保存の間」と伝えたか（Windows の実際の窓でなくても、伝える側の状態を確かめる）。
    #[doc(hidden)]
    pub fn saving_marked(&self) -> bool {
        self.saving_marked
    }

    /// 帯の右端のボタンと窓の縁を自前にするか（Windows の実際の窓は true。試験は Linux でも Windows の帯を描いて確かめる）。
    pub fn set_custom_frame(&mut self, on: bool) {
        self.custom_frame = on;
    }

    pub fn link(&self) -> &LiveLink {
        &self.link
    }

    /// Live Link（試験で受け渡しのフォルダを替える）。
    pub fn link_mut(&mut self) -> &mut LiveLink {
        &mut self.link
    }

    /// 外からの操作の受け口（試験が待っている番号・保存の返事待ちを見る）。
    pub fn ops(&self) -> &McpServer {
        &self.ops
    }

    /// 試験用: 外からの操作の受け口（1 フレームの数を小さくする）。
    #[doc(hidden)]
    pub fn ops_mut(&mut self) -> &mut McpServer {
        &mut self.ops
    }

    /// Live Link を始める・やめるの頼みと、ファイルの窓の頼みを当てる。
    fn handle_requests(&mut self) {
        if let Some(request) = self.state.link_request.take() {
            self.link.request(request, &mut self.state);
            self.state.link = self.link.view(&self.state);
        }
        // Live Link の違う相手の頼み: 保存していない変更を捨ててよいか（「開く」と同じ確かめ）
        if self.link.wants_discard() {
            let discard = self.confirm_discard();
            self.link.answer_discard(discard);
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
                    self.state
                        .apply(Action::Project(crate::newproject::NpAction::OpenNew));
                }
            }
            Some(DialogRequest::ProjectModel) => {
                let lang = self.state.lang;
                if let Some(path) = crate::dialog::file()
                    .set_title(lang.pick("モデルを選ぶ", "Choose a model"))
                    .add_filter("FBX", &["fbx", "FBX"])
                    .pick_file()
                {
                    self.state
                        .apply(Action::Project(crate::newproject::NpAction::ChooseModel(
                            path,
                        )));
                }
            }
            Some(DialogRequest::Open) => {
                let lang = self.state.lang;
                if self.confirm_discard() {
                    if let Some(path) = crate::dialog::file()
                        .set_title(lang.pick("プロジェクトを開く", "Open Project"))
                        .add_filter(
                            lang.pick("YoluPainter プロジェクト", "YoluPainter Project"),
                            &["ylp"],
                        )
                        .pick_file()
                    {
                        self.state.apply(Action::OpenProject(path));
                    }
                }
            }
            Some(DialogRequest::SaveAs) => {
                let lang = self.state.lang;
                let name = format!("{}.ylp", self.state.project_name);
                if let Some(path) = crate::dialog::file()
                    .set_title(lang.pick("別名で保存", "Save As"))
                    .add_filter(
                        lang.pick("YoluPainter プロジェクト", "YoluPainter Project"),
                        &["ylp"],
                    )
                    .set_file_name(name)
                    .save_file()
                {
                    self.state.apply(Action::SaveProjectAs(path));
                }
            }
            Some(DialogRequest::OpenModel) => {
                let lang = self.state.lang;
                if let Some(path) = crate::dialog::file()
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
                | DialogRequest::ShelfRemove
                | DialogRequest::LibraryAdd
                | DialogRequest::LibraryRemove
                | DialogRequest::LibraryReveal),
            ) => assets::run_dialog(&mut self.state, request),
            Some(DialogRequest::ExportFolder(id)) => {
                let lang = self.state.lang;
                let mut dialog = crate::dialog::file().set_title(
                    lang.pick("画像を書き出すフォルダ", "Folder for the exported images"),
                );
                // Live Link の相手の文書は、Unity が知らせた置き場（利用者が選び直したらそちら）から。無ければ、ある一番近い親から
                // （窓を取り消しても、Unity のプロジェクトにフォルダを残さないよう、ここでは作らない）
                if let Some(start) = self.state.link_export_start() {
                    dialog = dialog.set_directory(start);
                }
                if let Some(dir) = dialog.pick_folder() {
                    self.state.note_export_dir(&dir);
                    self.state
                        .apply(Action::Export(crate::export::ExportAction::TemplateTo {
                            id,
                            dir,
                        }));
                }
            }
            Some(DialogRequest::ExportChannel) => {
                let lang = self.state.lang;
                let mut dialog = crate::dialog::file()
                    .set_title(
                        lang.pick("チャンネルを PNG に書き出す", "Export the channel as PNG"),
                    )
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
                let mut dialog = crate::dialog::file()
                    .set_title(lang.pick("ライブラリの場所", "Library folder"));
                if let Some(current) = self
                    .state
                    .prefs
                    .settings
                    .library_folder()
                    .filter(|d| d.is_dir())
                {
                    dialog = dialog.set_directory(current);
                }
                if let Some(dir) = dialog.pick_folder() {
                    self.state
                        .apply(Action::Prefs(crate::prefs::PrefsAction::Set(
                            crate::prefs::Pref::LibraryFolder(Some(dir)),
                        )));
                }
            }
            Some(DialogRequest::PrefsCacheFolder) => {
                let lang = self.state.lang;
                let mut dialog =
                    crate::dialog::file().set_title(lang.pick("キャッシュの場所", "Cache folder"));
                let current = self.state.prefs.settings.disk_cache_folder();
                if current.is_dir() {
                    dialog = dialog.set_directory(current);
                }
                if let Some(dir) = dialog.pick_folder() {
                    self.state
                        .apply(Action::Prefs(crate::prefs::PrefsAction::Set(
                            crate::prefs::Pref::DiskCacheFolder(Some(dir)),
                        )));
                }
            }
            Some(DialogRequest::ExportChannelsFolder) => {
                let lang = self.state.lang;
                if let Some(dir) = crate::dialog::file()
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
                    if let Some(path) = crate::dialog::file()
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
                let name = crate::psd::default_export_name(&self.state);
                let mut dialog = crate::dialog::file()
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
            Some(DialogRequest::DistributeSave) => crate::distribute::run_dialog(&mut self.state),
            Some(DialogRequest::OpenStencil) => {
                let lang = self.state.lang;
                if let Some(path) = crate::dialog::file()
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
                if let Some(path) = crate::dialog::file()
                    .set_title(lang.pick(
                        "画像をアセットへ取り込む",
                        "Add an image to the project's assets",
                    ))
                    .add_filter("PNG", &["png", "PNG"])
                    .pick_file()
                {
                    self.state
                        .apply(Action::Fill(crate::fillfx::FillOp::ImportImage(path)));
                }
            }
            Some(DialogRequest::NewFillImage(mode)) => {
                let lang = self.state.lang;
                // 選ばずに閉じたら何も作らない（Undo の段も増やさない）
                if let Some(path) = crate::dialog::file()
                    .set_title(lang.pick("画像で塗りつぶしを作る", "Create a fill from an image"))
                    .add_filter("PNG", &["png", "PNG"])
                    .pick_file()
                {
                    self.state
                        .apply(Action::LayerMenu(crate::layermenu::Op::FillImageFile {
                            path,
                            mode,
                        }));
                }
            }
            Some(DialogRequest::ImportBrushes) => {
                let lang = self.state.lang;
                if let Some(paths) = crate::dialog::file()
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
            Some(DialogRequest::ClipStudioFolder) => {
                let lang = self.state.lang;
                let mut dialog = crate::dialog::file().set_title(lang.pick(
                    "CLIP STUDIO のサブツールのフォルダ",
                    "CLIP STUDIO sub tool folder",
                ));
                // 今探している場所（手で選んだフォルダか、既定の場所のうち開けたもの）から選び始める
                let csp = &self.state.brushes.csp;
                let start = csp.folder.clone().or_else(|| {
                    csp.listing
                        .as_ref()
                        .and_then(|l| l.searched.first().cloned())
                });
                if let Some(dir) = start.filter(|d| d.is_dir()) {
                    dialog = dialog.set_directory(dir);
                }
                if let Some(dir) = dialog.pick_folder() {
                    self.state.apply(Action::Brush(
                        crate::brushes::BrushAction::ClipStudioFolder(dir),
                    ));
                }
            }
            None => {}
        }
    }

    /// 保存していない変更があっても終わってよいか（窓を開かない試験では聞かない）。保存の途中には聞かない（保存が終わってから、その後の
    /// 状態で聞く）。
    fn confirm_close(&mut self) -> bool {
        // 更新のために終わるときは、保存するか捨てるかを更新の窓で選び済み。GPU の装置を失って終わるときは、復旧の書き置きに任せる
        if self.state.update.is_quitting() || self.gpu_lost.is_some() {
            return true;
        }
        // 閉じると取り消される仕事（利用者が結果を待っている書き出しなど）も、保存していない変更と一緒に知らせる
        let jobs = crate::windows::close_jobs(&self.state);
        if !self.state.modified && jobs.is_empty() {
            return true;
        }
        // 試験が、窓を開かずに答える口
        if let Some(answer) = self.close_answer.as_mut() {
            return answer(&self.state);
        }
        if !self.dialogs {
            return true;
        }
        crate::dialog::message()
            .set_title("YoluPainter")
            .set_description(crate::windows::close_question(
                self.state.lang,
                self.state.modified,
                &jobs,
            ))
            .set_buttons(rfd::MessageButtons::YesNo)
            .set_level(rfd::MessageLevel::Warning)
            .show()
            == rfd::MessageDialogResult::Yes
    }

    /// 保存していない変更を捨ててよいか（窓を開かない試験では、聞かずに捨てる）。保存の途中は、保存の結果が出るまで、頼む前の印のまま
    /// 聞く（保存の頼みは「変更あり」を下ろすが、保存が失敗すれば戻る。その変更を黙って捨てない）。
    fn confirm_discard(&self) -> bool {
        if !self.state.shows_modified() || !self.dialogs {
            return true;
        }
        crate::dialog::message()
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

    /// 窓に落としたファイル（.ylp なら開く。.psd なら新しいテクスチャセットとして取り込む。ブラシのファイル（ABR・GBR・GIH・VBR・PAT）なら
    /// 取り込む。PNG はブラシの一覧の上に落としたときだけブラシの筆先として取り込む）。
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
                self.state.refuse(
                    crate::notice::Source::Open,
                    crate::lang::refusals::during_stroke(self.state.lang),
                );
            } else if self.confirm_discard() {
                self.state.apply(Action::OpenProject(path.clone()));
            }
            return;
        }
        self.import_dropped_psd(&dropped);
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

    /// 落とした .psd を、「ファイル → 読み込み → PSD を新しいテクスチャセットへ」と同じ取り込み（読み込み → 取り込みの確かめの窓）へ回す。
    /// 取り込むのは最初の 1 つだけ（取り込みの確かめの窓は 1 つずつ。ほかは取り込まず、数を知らせる）。描いている最中は、取り込み側が断る。
    /// 行き先は新しいセット: 今のセットの文書を替えると取り消せないので、落としただけでは今の絵に触れない
    /// （文書を替えるときの、保存していない変更の確認はいらない）。
    fn import_dropped_psd(&mut self, dropped: &[std::path::PathBuf]) {
        let mut psds = dropped
            .iter()
            .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("psd")));
        let Some(first) = psds.next() else {
            return;
        };
        let more = psds.count();
        let busy = self.state.psd.is_busy() || self.state.psd.import_check.is_some();
        self.state.apply(Action::Psd(crate::psd::PsdAction::Import {
            path: first.clone(),
            target: crate::psd::PsdTarget::NewSet,
        }));
        // 取り込みが始まったときだけ（始まらなかった理由の文を、取り込まない件数で上書きしない）。理由は、仕事が終わったときの文に足す
        // （小さな PSD は同じフレームのうちに読み終わるので、読み始めの文に足しても残らない）
        if more > 0 && !busy && self.state.psd.is_busy() {
            self.psd_drop_more = more;
        }
    }

    /// 取り込まなかった PSD の件数を、取り込みの仕事の終わりの文に理由として足す（文の最初の 1 文はそのまま。断りの文と取り違えない）。
    fn note_dropped_psds(&mut self) {
        if self.psd_drop_more == 0 || self.state.psd.is_busy() {
            return;
        }
        let more = std::mem::take(&mut self.psd_drop_more);
        let lang = self.state.lang;
        let message = &self.state.message;
        let joint = lang.pick(
            if message.ends_with('。') { "" } else { "。" },
            if message.ends_with('.') { " " } else { ". " },
        );
        // 取り込みの知らせに添える（取り込まなかった物がある: 気をつけること）
        let extra = lang.pick(
            format!("ほか {more} 件は取り込みません（PSD は 1 つずつ）。"),
            format!("{more} more not imported (one PSD at a time)."),
        );
        self.state.amend(
            crate::notice::Kind::Warning,
            crate::notice::Source::Psd,
            joint,
            &extra,
        );
    }

    /// 3D ビューを wgpu で描く（eframe・kittest の RenderState。None なら 3D は描けないと出す）。
    pub fn with_render_state(mut self, rs: Option<&eframe::egui_wgpu::RenderState>) -> YoluApp {
        self.renderer3d = rs.map(View3dRenderer::new);
        // アンチエイリアスに選べる数は、この機材が描き先に使える数だけ
        let supported = self
            .renderer3d
            .as_ref()
            .map(|r| r.supported_samples().to_vec())
            .unwrap_or_default();
        self.state.view3d.display.set_supported_samples(&supported);
        self.gpu_device = rs.map(|rs| rs.device.clone());
        // キャンバスの合成も同じ装置で（使えるときは GPU。使えなければ CPU の表示）
        self.display.attach_render_state(rs.cloned());
        // アダプターから GPU のメモリの量が分かれば、設定が配る予算に使う（設定のファイルを読んだあとなので、ここで入れる）
        self.state.prefs.gpu = rs.map_or_else(Default::default, |rs| {
            crate::gpu_memory::Adapter::detect(&rs.adapter.get_info())
        });
        self.apply_gpu_memory();
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
    pub fn read_canvas_gpu_display(
        &mut self,
        rect: crate::engine::Rect,
    ) -> Result<Vec<u8>, String> {
        self.display.read_gpu_display(rect)
    }

    /// キャンバスの GPU の常駐の予算（試験・計測用。既定は `canvas::gpu::RESIDENT_BUDGET`）。
    pub fn set_canvas_gpu_budget(&mut self, bytes: u64) {
        self.display.set_gpu_budget(bytes);
    }

    /// GPU のメモリの設定から配って、3D の絵・キャンバスの合成・棚へ入れた予算（試験・計測用）。
    pub fn gpu_budgets_applied(&self) -> crate::gpu_memory::Budgets {
        self.gpu_budgets_applied
    }

    /// キャンバスの GPU の常駐の予算（試験・計測用）。
    pub fn canvas_gpu_budget(&self) -> u64 {
        self.display.gpu().budget()
    }

    /// 3D の絵の全体の予算（試験・計測用。wgpu が無ければ None）。
    pub fn view3d_paint_budget(&self) -> Option<u64> {
        self.renderer3d.as_ref().map(|r| r.paint_budget())
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

    /// 試験用: 3D の面の描き先に使ってよいバイト数を決める（None で既定の、3D の絵の予算と同じ量）。多サンプルを下げる道を通す。
    pub fn view3d_set_target_budget(&mut self, bytes: Option<u64>) {
        if let Some(r) = &mut self.renderer3d {
            r.set_target_budget(bytes);
        }
    }

    /// 3D の面の描き先に使ってよいバイト数（試験・計測用。設定の合計の外の勘定。wgpu が無ければ None）。
    pub fn view3d_target_budget(&self) -> Option<u64> {
        self.renderer3d.as_ref().map(|r| r.target_budget())
    }

    /// 3D の面の描き先に機材が使えるサンプル数（昇順。1 を含む。wgpu が無ければ None）。
    pub fn view3d_supported_samples(&self) -> Option<Vec<u32>> {
        self.renderer3d
            .as_ref()
            .map(|r| r.supported_samples().to_vec())
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

    /// 試験用: ほかのテクスチャセット（マテリアルの番号）の絵のチャンネルの 1 段の中身と、その縮めた段（絵を持っていなければ None）。
    pub fn view3d_read_other_level(
        &self,
        material: i32,
        slot: crate::view3d::paint::Slot,
        level: u32,
    ) -> Option<(Vec<u8>, [u32; 2], u32)> {
        self.renderer3d
            .as_ref()?
            .read_other_level(material, slot, level)
    }

    /// 試験用: 絵を持っているほかのテクスチャセットのマテリアル。
    pub fn view3d_held_materials(&self) -> Vec<i32> {
        self.renderer3d
            .as_ref()
            .map_or_else(Vec::new, |r| r.held_materials())
    }

    /// 試験用: ほかのテクスチャセットの絵を新しく作り始めてよい 1 フレームの時間（0 なら 1 フレームに 1 つ）。
    pub fn view3d_set_other_build_budget(&mut self, budget: std::time::Duration) {
        if let Some(r) = &mut self.renderer3d {
            r.set_other_build_budget(budget);
        }
    }

    /// 計測用: ほかのテクスチャセットの絵を見せるか（false は今のセットの絵だけを同期する、前の実装と同じ仕事）。
    pub fn view3d_set_show_other_sets(&mut self, show: bool) {
        if let Some(r) = &mut self.renderer3d {
            r.set_show_other_sets(show);
        }
    }

    /// 試験用: ほかのテクスチャセットの絵の辺の上限を小さくして、縮めの道を通す。
    pub fn view3d_set_other_cap(&mut self, cap: u32) {
        if let Some(r) = &mut self.renderer3d {
            r.set_other_cap(cap);
        }
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
        self.state
            .receive_link_model(model)
            .1
            .map_err(|e| e.to_string())
    }

    /// Live Link と同じ形のポーズを当てる（描いている最中なら、終わってから）。
    pub fn apply_live_link_pose(&mut self, pose: &yolu_protocol::Pose) -> Result<(), String> {
        self.state
            .receive_link_pose(pose)
            .map_err(|e| e.to_string())
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
        self.state.ui.canvas_rect
    }

    pub fn thumbnails(&self) -> &Thumbnails {
        &self.thumbs
    }

    /// 3D ビューの中身を描く外の窓の口を渡す。
    pub fn set_view3d_host(&mut self, host: Box<dyn View3dHost>) {
        self.view3d.set_host(host);
    }

    /// 1 フレーム（eframe と試験の両方がここを呼ぶ）。フレームの中で `message` に書かれた文（非同期の終わりなど、`apply` の外の書き込みも）は、
    /// 前と同じ文でも新しい知らせとして出る。
    pub fn frame(&mut self, ui: &mut Ui) {
        let prior = self.state.message_begin();
        self.frame_body(ui);
        self.state.message_end(prior);
        self.finish_message(ui.ctx());
    }

    /// フレームの終わりの `message`: 失敗・断り・警告を記録へ（同じ文なら何もしない）。新しい知らせがあれば、すぐ出すための描き直しを頼む。
    fn finish_message(&mut self, ctx: &egui::Context) {
        self.state.record_message();
        if self.state.toast.is_pending() {
            ctx.request_repaint();
        }
    }

    fn frame_body(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        self.poll_gpu_watch(&ctx);
        self.state.ui.popup_was_open = self.state.popup.is_some();
        crate::region::bucket::poll(&mut self.state, &ctx);
        let mut pen = self.pen.drain();
        // 窓の縁（自前の枠だけ）: 押したら大きさを変える頼みを送る。描いている最中・ペンが触れている最中（キャンバスと 3D ビューが
        // ペンの押しとして扱うのと同じ `contact`。筆圧は触れていなくても 1 のペンも、触れた直後は 0 のペンもある）は受けない
        let edge = self.custom_frame.then(|| {
            titlebar::edges(
                &ctx,
                self.state.is_stroking() || pen.iter().any(|s| s.contact),
                &self.bar_press_rects,
            )
        });
        // 筆圧の調整の窓: 調整を通す前の筆圧を集め、そのあとで全体の調整（設定）を通してから、キャンバスと 3D ビューへ渡す
        self.state.pressure_observe(ctx.pixels_per_point(), &pen);
        // 窓が開いているときだけ（描いている間じゅう毎フレーム、全イベントの写しを作らない）
        if self.state.pressure.open && !self.pen.is_hooked() {
            ctx.input(|i| self.state.pressure_observe_touch(&i.events));
        }
        for sample in &mut pen {
            sample.pressure = self.state.adjust_pressure(sample.pressure);
        }
        shell::handle_shortcuts(&ctx, &mut self.state);
        crate::stencil::update_keys(&ctx, &mut self.state);
        self.open_dropped(&ctx);
        // 一覧の範囲はこのフレームで描いたときだけ入る（棚・チャンネルのタブを開いている間に、前の位置へ落とした PNG を取り込まない）
        self.state.brushes.ui.list_rect = None;
        assets::frame(&ctx, &mut self.state);
        self.handle_requests();
        self.link.poll(&mut self.state);
        self.state.link = self.link.view(&self.state);
        // Live Link の頼みは描き直しの頼みが無くても拾う（受け付けている間は inbox を見る間隔で回す）
        if let Some(wake) = self.link.next_wake() {
            ctx.request_repaint_after(wake);
        }
        // 別のスレッドの仕事（ベイク・書き出し・PSD）の終わりを受ける
        self.state.poll_bake();
        self.state.poll_export();
        self.state.sync_budgets();
        self.state.check_tile_cache();
        self.state.poll_psd();
        self.note_dropped_psds();
        self.state.poll_distribute();
        self.poll_saving();
        // 外からの操作: 設定に合わせて待ち受けを始める・やめ、受けた要求を実行する（保存の結果を受けた後に。返事待ちの保存の返事も返す）
        self.tick_ops(&ctx);
        self.state.poll_brush_import();
        self.state.poll_brush_csp();
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
        crate::panels::pose::ensure_tab(&self.state, &mut self.dock);
        if self.state.reset_layout {
            self.dock = default_dock();
            self.state.reset_layout = false;
        }

        // 状態の帯の右端のメモリ（実際の窓だけ。1.5 秒おきに測り、止まっていても同じ間隔で描き直す）
        if self.dialogs {
            let now = ctx.input(|i| i.time);
            let device = self.gpu_device.clone();
            self.state.refresh_usage(now, || {
                device
                    .and_then(|d| d.generate_allocator_report())
                    .map(|report| report.total_allocated_bytes)
            });
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(crate::usage::INTERVAL));
        }
        let mut bar = None;
        let mut link_icon = None;
        let custom_frame = self.custom_frame;
        let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
        let mut frame_commands = Vec::new();
        let mut caption = None;
        egui::Panel::top("yolu.menubar")
            .exact_size(t::MENU_BAR_HEIGHT)
            .frame(Frame::NONE)
            .show(ui, |ui| {
                let r = ui.max_rect();
                // 自前の枠: 右端の 3 つのボタンの左までが帯の中身。何も無い所は、窓を動かす・最大化する部品（メニューの見出しなどより先に作る）
                let content = titlebar::content_rect(r, custom_frame);
                let drag = custom_frame.then(|| titlebar::drag_zone(ui, content));
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
                    self.state.update.offer().map(|_| shell::HELP_MENU),
                ));
                // 右端: プロジェクトの名前と保存の状態。その左に Live Link の入口（Unity の印）。名前は、メニューの見出しの右から窓の右の縁までの
                // 幅（最大 352）に収まるように後ろを詰め、印は「見えている名前」の左に置く（長い名前でも、印がメニューの見出しに重ならない）
                let menu_end = bar
                    .as_ref()
                    .and_then(|b| b.rects.last())
                    .map_or(r.left() + 6.0, |last| last.right());
                let room =
                    (content.right() - 8.0 - (menu_end + 6.0 + shell::LINK_ICON_SLOT + 28.0))
                        .clamp(0.0, 352.0);
                let style = t::LABEL_DIM.with_color(if self.state.shows_modified() {
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
                    pos2(content.right() - 8.0 - name_width, r.top()),
                    pos2(content.right() - 8.0, r.bottom()),
                );
                let name = format!(
                    "{shown}{}",
                    if self.state.shows_modified() {
                        " •"
                    } else {
                        ""
                    }
                );
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
                    content.right() - 8.0 - name_width,
                    &self.state,
                    link_open,
                ));
                if let Some(drag) = drag {
                    // 自分の押しを持つ部品（メニューの見出し・クラッシュと Live Link の印）の上の押しは、帯の操作にしない
                    let mut blockers = bar.as_ref().map(|b| b.rects.clone()).unwrap_or_default();
                    blockers.push(crash_rect);
                    blockers.extend(link_icon.map(|i| i.rect));
                    self.bar_press_rects = link_icon.map(|i| i.rect).into_iter().collect();
                    frame_commands = titlebar::drag_commands(&drag, &blockers, maximized);
                    caption = titlebar::buttons(ui, r, maximized, self.state.lang);
                }
            });
        for command in frame_commands {
            ctx.send_viewport_cmd(command);
        }
        match caption {
            // 閉じるは、メニューの「終了」と同じ道（保存していない変更の確かめ。下の終了の処理が受ける）
            Some(titlebar::Button::Close) => self.state.apply(Action::Quit),
            Some(button) => {
                if let Some(command) = button.command(maximized) {
                    ctx.send_viewport_cmd(command);
                }
            }
            None => {}
        }
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
                self.state.ui.dock_grab = [grabbed, self.state.ui.dock_grab[0]];
            });
        // 3D ビューのタブが見えているか（次のフレームのキー入力・メニューの取り消しの行き先が読む）
        self.state.view3d.visible = self.view3d.content_rect().is_some();
        self.state.ui.canvas_visible = std::mem::take(&mut self.state.ui.canvas_drawn);
        // 隠れたビューは、ペンが離れたのを受け取れない（タブの見出しをつかんで動かしているあいだなど）。ペンの押しの印と、ペンが回し・
        // パン・拡縮していた途中を、見えるようになるまで持ち越さない（印が残ると、ペンの押しとみなしてマウスの押しを使わなくなる）
        if !self.state.ui.canvas_visible {
            self.state.drafting_cancel();
            self.state.drafting.pen_down = None;
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
        // 窓の縁の上のポインタの形（キャンバスなどが決めた形を上書きする）
        if let Some(direction) = edge.flatten() {
            titlebar::edge_cursor(&ctx, direction);
        }
        crate::selection::dialog::show(&ctx, &mut self.state);
        crate::windows::show(&ctx, &mut self.state);
        crate::prefs::show(&ctx, &mut self.state);
        crate::pen::window::show(&ctx, &mut self.state);
        crate::recovery::window::show(&ctx, &mut self.state);
        // 色の窓（相手の欄はこのフレームに描いた。変更は次のフレームに相手が受け取る）
        crate::panels::color_window::show_in_app(&ctx, &mut self.state);
        self.state.crash.show(&ctx, self.state.lang);
        // 直前の操作の知らせ（状態の帯の左には出さず、短く出して消える）
        crate::toast::show(&ctx, &mut self.state);
        if self.dialogs {
            self.state.crash.execute_request(self.state.lang);
        }
        let popup_rect = self.state.popup.as_ref().map(|p| p.state.rect);
        self.view3d.end_frame(popup_rect);
        // メニューで選んだ Live Link・ファイルの頼みはこのフレームのうちに当てる
        self.handle_requests();
        self.link_exported();
        self.state.link = self.link.view(&self.state);
        // 「保存して更新」: 保存先を選ぶ窓も済んだこのフレームの終わりに、保存の結果を見て入れる
        self.state.update_finish_save();
        // 終了・窓を閉じる: 保存していない変更があれば聞く（窓を開かない試験では聞かない）
        let close_requested = ctx.input(|i| i.viewport().close_requested());
        self.close_flow(&ctx, close_requested);
    }

    /// 保存の結果を受けて（フレームの初め）、OS の終了を待たせる印を今の保存の有無に合わせる。窓が見えている間（`ui`）も隠れている間
    /// （`logic`）も、保存の結果を受ける所はこれを通す。
    fn poll_saving(&mut self) {
        self.state.poll_save();
        self.mark_saving();
    }

    /// OS の終了を待たせる印を、今の保存の有無に合わせる（変わったときだけ OS へ伝わる）。
    fn mark_saving(&mut self) {
        let saving = self.state.is_saving();
        self.saving_marked = saving;
        crate::session_end::set_saving(
            saving,
            self.state
                .lang
                .pick("YoluPainter が保存しています", "YoluPainter is saving"),
        );
    }

    /// 終了・窓を閉じる頼みを進める。保存の途中は閉じず（保存を捨てない）、終わるまで待つ。保存が終わったら、その結果の後の状態で、
    /// 保存していない変更があれば聞き、走っている仕事の後始末をして閉じる。
    fn close_flow(&mut self, ctx: &egui::Context, close_requested: bool) {
        if (!self.state.quit && !close_requested) || self.closing {
            return;
        }
        if self.state.is_saving() {
            // 窓を閉じる頼みは止めて、終わるまで待つ（`quit` に覚える）。画面のスレッドは回し続ける（「応答なし」にならない）
            if close_requested {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.state.quit = true;
            }
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        } else if self.confirm_close() {
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

    fn popups(
        &mut self,
        ctx: &egui::Context,
        bar: &menu::BarOutcome,
        link_icon: Option<shell::LinkIcon>,
    ) {
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
            if matches!(
                self.state.popup.as_ref().map(|p| p.kind),
                Some(PopupKind::LiveLink)
            ) {
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
fn startup_project(args: impl Iterator<Item = std::ffi::OsString>) -> Option<std::path::PathBuf> {
    let path = std::path::PathBuf::from(args.skip(1).find(|arg| arg != "--livelink")?);
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("ylp"))
        .then_some(path)
}

/// 起動時の状態の帯の知らせ（既定へ戻した設定の理由。あれば全部）。ペンを受けていることは知らせない（状態の文を帯に出さない）。
fn startup_message(lang: crate::lang::Lang, problems: &[Problem]) -> Option<String> {
    let parts: Vec<String> = problems.iter().map(|p| p.text(lang)).collect();
    (!parts.is_empty()).then(|| parts.join(" "))
}

impl YoluApp {
    /// Live Link を 1 回まわす: Unity からの頼みを拾って当て、書き出しの返事を書く。窓が見えている間はフレームの中（`frame_body`）が
    /// 同じことをするので、これを呼ぶのは窓が隠れている間だけ（`eframe::App::logic`）。
    fn tick_link(&mut self) {
        self.link.poll(&mut self.state);
        self.link_exported();
        self.state.link = self.link.view(&self.state);
    }

    /// 書き出しが終わっていれば、Live Link の相手へ `exported` の返事を書く。
    fn link_exported(&mut self) {
        if let Some(files) = self.state.export.take_finished() {
            self.link.exported(&mut self.state, &files);
        }
    }

    /// 外からの操作を 1 回まわす: 設定（外からの操作を受ける）に合わせて待ち受けを始める・やめ、受けた要求を画面のスレッドで実行して返す。
    fn tick_ops(&mut self, ctx: &egui::Context) {
        let want = self.state.prefs.settings.external_ops;
        let port = self.state.prefs.settings.external_ops_port;
        self.ops.sync(want, port, ctx, &mut self.state);
        self.ops.poll(&mut self.state);
        self.state.ops = self.ops.view();
    }

    /// 窓が隠れている間の 1 回（`eframe::App::logic` が、見えていないときに呼ぶ。試験は、`ui` を回さずにこれを呼んで、隠れた窓の道を
    /// 通す）。
    #[doc(hidden)]
    pub fn tick_hidden(&mut self, ctx: &egui::Context) {
        // 新しい知らせの扱いは `ui` と同じ（隠れている間に出た文は、見えるようになった最初のフレームで知らせとして出る。ここで描き直しは頼まない:
        // 見えない窓を知らせのために回し続けない）
        let prior = self.state.message_begin();
        self.poll_gpu_watch(ctx);
        self.tick_link();
        // 見えない窓でも、Live Link の頼みを拾う間隔で回す（受け付けている間だけ）
        if let Some(wake) = self.link.next_wake() {
            ctx.request_repaint_after(wake);
        }
        // 保存の途中は、隠れていても保存を捨てて閉じない。窓を閉じる頼み（タスクバーの「閉じる」など）は止めて待ち、終わりを受け、
        // 保存が終わって終了の頼みが残っていれば閉じる流れを進める（見えない窓の保存を、知らせのために回し続けはしない: 保存の間だけ）
        if self.state.is_saving() {
            let close_requested = ctx.input(|i| i.viewport().close_requested());
            self.close_flow(ctx, close_requested);
            self.poll_saving();
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        } else if self.state.quit {
            self.close_flow(ctx, false);
        }
        // 見えない窓でも、受けた要求は実行する（保存の結果を受けた後に）
        self.tick_ops(ctx);
        self.state.message_end(prior);
        self.state.record_message();
    }
}

impl eframe::App for YoluApp {
    /// 窓が隠れている間（Windows の最小化・macOS の覆われた窓・隠した窓）は、eframe は egui のパスを回さず `ui` を呼ばない。代わりに、
    /// 描き直しの頼みがあるときだけ（百ミリ秒より速くならない）この `logic` を呼ぶ（eframe 0.36 の `App::logic`）。Live Link の裏のスレッドは
    /// Unity からの知らせのたびに描き直しを頼むので、ここで受け取り・返事をすれば、最小化したまま Unity で Play に入る・スクリプトを
    /// リロードしても、再接続と絵の受け渡しが続く。画面に触れる処理（描く・並べる）は `ui` のまま。見えている間は `ui` がするので何もしない。
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if ctx.input(|i| i.viewport().visible()) != Some(false) {
            return;
        }
        self.tick_hidden(ctx);
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        // フレームの外側（拾った色・合成・設定と並びの保存）が書いた文も、前と同じ文でも新しい知らせにする
        let prior = self.state.message_begin();
        crate::screen_pick::frame(&mut self.state, _frame);
        self.apply_compositing();
        self.apply_gpu_memory();
        self.frame(ui);
        // このフレームの中で始めた保存も、次のフレームを待たずに OS の終了を待たせる印へ伝える
        self.mark_saving();
        // 押していないのに残った 3D の塗りの切り替えのドラッグの印は下ろす（欄が描かれなくなった間に離したとき）
        if !ui.ctx().input(|i| i.pointer.any_down()) {
            self.state.view3d.projection_dragging = false;
        }
        self.persist_settings();
        self.persist_layout(ui.ctx());
        self.state.message_end(prior);
        self.finish_message(ui.ctx());
    }

    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        self.remap_pen_buttons(&mut raw_input.events);
    }

    /// 正しく終わった: 変更があれば最後の世代を書き、復旧の印を消す（世代は設定の数だけ残す）。
    fn on_exit(&mut self) {
        if self.gpu_lost.is_some() && self.state.modified {
            // 装置を失って終わる: 書き置きの「保存していない作業」の印を消さず（落ちたときと同じ）、書き込み中の分だけ、期限まで待つ。
            // 遅いディスクで間に合わなくても固まらない（置換は最後の 1 回なので、前の世代が残る）。次の起動の復旧の窓から開ける
            self.state.recovery_wait_within(self.gpu_lost_wait);
        } else {
            self.state.recovery_shutdown();
        }
        // 並びと窓の大きさ・位置を、終わるときに書く（途中で書けていなくても、最後の形を残す）
        self.save_layout(false);
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        egui::Rgba::from(t::WINDOW_BG).to_array()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lang::Lang;

    fn exchange_folder(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("yl-start-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("LiveLink")
    }

    #[test]
    fn live_link_startup_obeys_settings_and_explicit_launch_flag() {
        let ctx = egui::Context::default();
        for (i, (enabled, flag, expected)) in [
            (true, false, true),
            (false, false, false),
            (false, true, true),
            (true, true, true),
        ]
        .into_iter()
        .enumerate()
        {
            let mut app = YoluApp::with_state(AppState::new(64, 64), PenInput::detached());
            let root = exchange_folder(&i.to_string());
            app.link.set_folder(root.clone()).unwrap();
            app.state.prefs.settings.livelink_on_startup = enabled;
            app.state.message = "起動時の知らせ".into();
            let mut args = vec![std::ffi::OsString::from("yolupainter")];
            if flag {
                args.push("--livelink".into());
            }
            app.start_live_link(&ctx, args.into_iter());
            assert_eq!(app.state.link.is_on(), expected);
            assert_eq!(app.state.message, "起動時の知らせ");
            assert_eq!(
                root.join("presence.json").is_file(),
                expected,
                "起きている印"
            );
            drop(app);
            assert!(!root.join("presence.json").exists(), "終わると印を消す");
            let _ = std::fs::remove_dir_all(root.parent().unwrap());
        }
    }

    #[test]
    fn live_link_startup_with_an_unusable_folder_shows_failure() {
        let ctx = egui::Context::default();
        let root = exchange_folder("file");
        std::fs::create_dir_all(root.parent().unwrap()).unwrap();
        std::fs::write(&root, b"not a folder").unwrap();
        let mut app = YoluApp::with_state(AppState::new(64, 64), PenInput::detached());
        app.link.set_folder(root.clone()).unwrap();
        app.start_live_link(&ctx, ["yolupainter"].into_iter().map(Into::into));
        assert!(matches!(
            app.state.link.status,
            crate::livelink::LinkStatus::Failed(_)
        ));
        assert!(app.state.link.tooltip(Lang::Ja).lines().count() >= 2);
        let _ = std::fs::remove_dir_all(root.parent().unwrap());
    }

    #[test]
    fn live_link_flag_preserves_project_arguments_in_either_order() {
        for args in [
            vec!["yolupainter", "--livelink", "sample.ylp"],
            vec!["yolupainter", "sample.ylp", "--livelink"],
        ] {
            assert_eq!(
                startup_project(args.into_iter().map(Into::into)),
                Some("sample.ylp".into())
            );
        }
        assert_eq!(
            startup_project(["yolupainter", "--livelink"].into_iter().map(Into::into)),
            None
        );
    }

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
        let mut app =
            YoluApp::with_state(crate::state::AppState::new(64, 64), PenInput::detached());
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
        assert_eq!(
            app.state.project.as_ref().map(|p| p.path().to_path_buf()),
            Some(project.clone())
        );
        assert!(
            !app.state.message.contains("開けません"),
            "{}",
            app.state.message
        );
        // 壊れた .ylp・無い .ylp は、開かずに理由を知らせる（元の文書はそのまま）
        let broken = dir.join("Broken.ylp");
        std::fs::write(&broken, b"not a project").unwrap();
        for bad in [broken, dir.join("missing.ylp")] {
            let app = startup_app(&[&bad]);
            assert!(app.state.project.is_none(), "{}", bad.display());
            assert!(
                app.state.message.contains("を開けません（"),
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
        assert_eq!(
            startup_message(Lang::En, &[Problem::Unreadable]).as_deref(),
            Some("Cannot read the settings.")
        );
        assert_eq!(
            startup_message(Lang::En, &[Problem::Language("x".into())]).as_deref(),
            Some("Cannot read the language setting.")
        );
        // 読めなかった設定の理由が 2 つ以上なら全部
        let both = startup_message(
            Lang::Ja,
            &[
                Problem::Language("x".into()),
                Problem::Invalid {
                    key: "cpu_threads",
                    value: "0".into(),
                },
                Problem::Backups("-2".into()),
            ],
        )
        .unwrap();
        assert!(
            both.contains("言語の設定を読めません")
                && both.contains("CPU のスレッド")
                && both.contains("退避を残す数")
                && !both.contains("Windows Ink"),
            "{both}"
        );
    }
}
