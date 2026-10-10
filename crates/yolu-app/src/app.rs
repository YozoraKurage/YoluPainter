//! アプリの本体: 外枠（メニューバー・オプションバー・ツールの帯・ステータスバー）と、egui_dock のドッキング（Substance の並び。
//! 左: アセットとカラー、中央: キャンバスと 3D ビュー、右: テクスチャセットとレイヤーとプロパティ）、ポップアップ、3D ビューの知らせ、
//! Live Link（フレームの頭で受け、終わりに変わったタイルを出す）、ファイルのウィンドウ。

use std::collections::HashMap;

use egui::{pos2, vec2, Color32, Frame, Id, Rect, RichText, Sense, Ui, WidgetText};
use egui_dock::{DockArea, DockState, Node, NodeIndex, TabViewer};

use crate::canvas::{self, display::CanvasDisplay};
use crate::dialog::places::Place;
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
    /// サブツールの一覧（左のドックの先頭。中身は今のツールに合わせて替わる）。
    SubTools,
    /// ツールプロパティ（今のツールの設定の全部。描くツールには「塗るチャンネル」の区分も）。既定の並びではサブツールの下。
    /// 古い並びに無ければ、読んだときにサブツールの組の下へ足す。
    ToolProperties,
    /// ブラシサイズ（大きさを持つツールだけ。持たないツールでは空）。既定の並びではツールプロパティの下。古い並びに無ければ、読んだときにツールプロパティの下へ足す。
    BrushSize,
    Assets,
    Color,
    /// ポーズ（ボーンのインスペクター・BlendShape・面を隠す）。スキンのあるモデルを読むと、プロパティと同じ組へ足される。
    Pose,
    Canvas,
    View3d,
    TextureSets,
    Layers,
    Properties,
    /// マテリアル（今のテクスチャセットの見た目: 種類・ひな形・描画モード・lilToon の各設定）。既定の並びではプロパティ・ヒストリーと同じ組。
    /// 古い並びに無ければ、読んだときにプロパティの組の後ろへ足す。
    Material,
    Channels,
    History,
    ColorSets,
    Navigator,
    /// 起動してからの注意と失敗の知らせ（既定の並びではレイヤーと同じ組の後ろ。プロパティ・ヒストリーの組に 3 つ並べると、いちばん小さいウィンドウで
    /// 英語の名前が欠ける。古い並びに無ければ、読んだときにレイヤーの組へ足す）。
    Log,
    /// アクション（操作の記録と再生）。既定の並びには無く、「ウィンドウ」のメニューから開くと（`DockOp::Show`）レイヤーと同じ組の後ろへ入る。
    Actions,
}

impl Tab {
    /// ドックに出るタブ全部（ポーズは、スキンのあるモデルを読むと足される）。
    pub const ALL: [Tab; 18] = [
        Tab::SubTools,
        Tab::ToolProperties,
        Tab::BrushSize,
        Tab::Assets,
        Tab::Color,
        Tab::Pose,
        Tab::Canvas,
        Tab::View3d,
        Tab::TextureSets,
        Tab::Layers,
        Tab::Properties,
        Tab::Material,
        Tab::Channels,
        Tab::History,
        Tab::ColorSets,
        Tab::Navigator,
        Tab::Log,
        Tab::Actions,
    ];

    /// 保存する名前（並びのファイル `layout.json` に書く。Rust の名前を変えても変わらないよう、ここで決める。足すのは良いが、
    /// 書き換えると保存済みの並びが「知らないタブ」で捨てられる）。
    pub fn key(self) -> &'static str {
        match self {
            Tab::SubTools => "subtools",
            Tab::ToolProperties => "tool_properties",
            Tab::BrushSize => "brush_size",
            Tab::Assets => "assets",
            Tab::Color => "color",
            Tab::Pose => "pose",
            Tab::Canvas => "canvas",
            Tab::View3d => "view3d",
            Tab::TextureSets => "texture_sets",
            Tab::Layers => "layers",
            Tab::Properties => "properties",
            Tab::Material => "material",
            Tab::Channels => "channels",
            Tab::History => "history",
            Tab::ColorSets => "color_sets",
            Tab::Navigator => "navigator",
            Tab::Log => "log",
            Tab::Actions => "actions",
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
            Tab::ToolProperties => lang.pick("ツールプロパティ", "Tool Properties"),
            Tab::BrushSize => lang.pick("ブラシサイズ", "Brush Size"),
            Tab::Assets => lang.pick("アセット", "Assets"),
            Tab::Color => lang.pick("カラー", "Color"),
            Tab::Pose => lang.pick("ポーズ", "Pose"),
            Tab::Canvas => lang.pick("キャンバス", "Canvas"),
            Tab::View3d => lang.pick("3D ビュー", "3D View"),
            Tab::TextureSets => lang.pick("テクスチャセット", "Texture Sets"),
            Tab::Layers => lang.pick("レイヤー", "Layers"),
            Tab::Properties => lang.pick("プロパティ", "Properties"),
            Tab::Material => lang.pick("マテリアル", "Material"),
            Tab::Navigator => lang.pick("ナビゲーター", "Navigator"),
            Tab::Channels => lang.pick("チャンネル", "Channels"),
            Tab::History => lang.pick("ヒストリー", "History"),
            Tab::ColorSets => lang.pick("カラーセット", "Color Sets"),
            Tab::Log => lang.pick("ログ", "Log"),
            Tab::Actions => lang.pick("アクション", "Actions"),
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

/// 左の列の割合（上から サブツール・ツールプロパティ・ブラシサイズ・カラー。列全体に対して）。`default_dock_for` が使う。
pub(crate) const LEFT_SHARE: f32 = 0.21;
pub(crate) const LEFT_COLOR: f32 = 0.32;
pub(crate) const LEFT_TOOL_PROPERTIES: f32 = 0.35;
pub(crate) const LEFT_BRUSH_SIZE: f32 = 0.13;
/// 左の列の、サブツールの取り分（残り）。
pub(crate) const LEFT_SUB_TOOLS: f32 = 1.0 - LEFT_COLOR - LEFT_TOOL_PROPERTIES - LEFT_BRUSH_SIZE;
/// 右の列の幅: ウィンドウの幅の割合と、下限（点。右上の 3 つのタブ（テクスチャセット・チャンネル・アセット）の名前が日英とも切れない幅）。
/// 下限は、最小のウィンドウ（960 点）の幅で効く。
const RIGHT_WIDTH_SHARE: f32 = 0.19;
const RIGHT_WIDTH_MIN: f32 = 232.0;
/// 中央の 3D ビュー（左）の取り分。残りがキャンバス（右）。
pub(crate) const CENTER_VIEW3D: f32 = 0.5;
const RIGHT_TOP: f32 = 0.46;
/// 右の列の、上の組を除いた残りのうち、レイヤーの組の取り分（残りがプロパティの組）。
const RIGHT_LAYERS: f32 = 0.42;
/// 既定の並びを、幅の決まらない所（`default_dock`）で作るときのウィンドウの幅（起動の初めのウィンドウの幅）。
const REFERENCE_WIDTH: f32 = 1600.0;

/// 既定の並び（ウィンドウの幅は起動の初めの幅）。
pub fn default_dock() -> DockState<Tab> {
    default_dock_for(REFERENCE_WIDTH)
}

/// 幅 `width` 点のウィンドウでの既定の並び。Substance Painter の並び（左: サブツール・ツールプロパティ・ブラシサイズ・カラー、中央: 3D ビュー（左）と
/// キャンバス（右）、右: 上からテクスチャセットとチャンネルとアセット・レイヤーとログ・プロパティとマテリアルとヒストリー）。ナビゲーターは閉じている
/// （「ウィンドウ」のメニューで開くとテクスチャセットの組に入る）。右の列の幅は、ウィンドウの幅の `RIGHT_WIDTH_SHARE`、ただし `RIGHT_WIDTH_MIN` 点より狭くしない
/// （egui_dock の割合は幅に対する割合なので、幅から決める）。egui_dock の割合は左（上）の子の取り分（分けた向きによらない）。
pub fn default_dock_for(width: f32) -> DockState<Tab> {
    // ドックの幅は、ウィンドウの幅から左のツールの帯を除いた分
    let dock_width = (width - t::TOOL_STRIP_WIDTH).max(1.0);
    let left = LEFT_SHARE * dock_width;
    let right = (RIGHT_WIDTH_SHARE * width).max(RIGHT_WIDTH_MIN);
    // 左の列を除いた幅のうち、中央（3D ビューとキャンバス）の取り分
    let center_share = (1.0 - right / (dock_width - left).max(1.0)).clamp(0.3, 0.9);
    let mut dock = DockState::new(vec![Tab::Canvas]);
    let surface = dock.main_surface_mut();
    let [center, left] = surface.split_left(NodeIndex::root(), LEFT_SHARE, vec![Tab::SubTools]);
    let [center, right] = surface.split_right(
        center,
        center_share,
        vec![Tab::TextureSets, Tab::Channels, Tab::Assets],
    );
    surface.split_left(center, CENTER_VIEW3D, vec![Tab::View3d]);
    // 左の列は上から サブツール・ツールプロパティ・ブラシサイズ・カラー（それぞれが自分のタブの帯を持つ）
    let upper_share = 1.0 - LEFT_COLOR;
    let [upper, _] = surface.split_below(left, upper_share, vec![Tab::Color, Tab::ColorSets]);
    let sub_share = LEFT_SUB_TOOLS;
    let [_, props_and_size] =
        surface.split_below(upper, sub_share / upper_share, vec![Tab::ToolProperties]);
    surface.split_below(
        props_and_size,
        LEFT_TOOL_PROPERTIES / (LEFT_TOOL_PROPERTIES + LEFT_BRUSH_SIZE),
        vec![Tab::BrushSize],
    );
    let [_, layers] = surface.split_below(right, RIGHT_TOP, vec![Tab::Layers, Tab::Log]);
    surface.split_below(
        layers,
        RIGHT_LAYERS,
        vec![Tab::Properties, Tab::Material, Tab::History],
    );
    dock
}

/// ウィンドウの幅がこれ（点）より変わらなければ、自動の割合を直さない。
const DOCK_FIT_EPSILON_WIDTH: f32 = 0.5;
/// 割合が同じとみなす差。
const DOCK_FIT_EPSILON_FRACTION: f32 = 1e-4;

/// `tree` が `reference`（既定の並び）と同じ形か: ノードの数と種類（分け目の向き）が同じで、組ごとのタブが同じ（`reference` に無いタブ — ナビゲーター・アクション・
/// ポーズ — は数えない）。`check_fractions` なら、各分け目の割合も 1e-4 以内で同じ。
fn same_split_shape(
    tree: &egui_dock::Tree<Tab>,
    reference: &egui_dock::Tree<Tab>,
    check_fractions: bool,
) -> bool {
    if tree.len() != reference.len() {
        return false;
    }
    let known: Vec<Tab> = reference
        .iter()
        .flat_map(|n| n.iter_tabs().copied())
        .collect();
    tree.iter().zip(reference.iter()).all(|pair| match pair {
        (Node::Empty, Node::Empty) => true,
        (Node::Leaf(a), Node::Leaf(b)) => {
            let mine: Vec<Tab> = a
                .tabs
                .iter()
                .copied()
                .filter(|t| known.contains(t))
                .collect();
            mine == b.tabs
        }
        (Node::Horizontal(a), Node::Horizontal(b)) | (Node::Vertical(a), Node::Vertical(b)) => {
            !check_fractions || (a.fraction - b.fraction).abs() <= DOCK_FIT_EPSILON_FRACTION
        }
        _ => false,
    })
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

/// ドックの描き方（メインウィンドウと別ウィンドウで同じ）。タブは閉じない（閉じるとキャンバスを失う）・足すボタンを出さない・右クリックは
/// アプリのメニュー（egui_dock の組み込みのメニューは切る）。
fn dock_area(dock: &mut DockState<Tab>, id: Id, style: egui_dock::Style) -> DockArea<'_, Tab> {
    // 浮かせたウィンドウの閉じるボタンは出さない。egui_dock 0.21 は代わりの名前を案内するが、その名前の関数はまだ無いので、古い名前を使う
    #[allow(deprecated)]
    let area = DockArea::new(dock).show_window_close_buttons(false);
    area.id(id)
        .style(style)
        .show_close_buttons(false)
        .show_add_buttons(false)
        .show_leaf_collapse_buttons(false)
        .show_leaf_close_all_buttons(false)
        .tab_context_menus(false)
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
    /// このフレームに見出しを引いて離したタブ（離した所がウィンドウの外なら、別ウィンドウへ出す・別のウィンドウへ移す）。
    released: Option<Tab>,
    /// このフレームに見出しを右クリックしたタブと、押した点。
    context: Option<(Tab, egui::Pos2)>,
    /// タブを浮いたウィンドウの落とし先にしてよいか。別ウィンドウのタブがただ 1 つのときは、組の中ほど以外で離すと、そのタブだけの
    /// 新しいウィンドウに替わって元のウィンドウが空になる（同じウィンドウを作り直すだけ）ので、落とし先にしない。
    windows_allowed: bool,
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
        // 前のタブ（キャンバス・3D ビュー）の押しで描き始めたなら、このタブからは描き始める前の見た目を使う
        w::update_stroke_hold(ui.ctx(), self.app.holds_panel_look());
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
            Tab::Color if !self.app.mode.paints() => {
                let rect = ui.max_rect();
                w::enabled_scope(ui, "mode.dim.color", false, |ui| {
                    crate::panels::color::show(ui, self.app, self.colors)
                });
                crate::mode::dimmed_reason(ui, self.app, rect, "color");
            }
            Tab::Color => crate::panels::color::show(ui, self.app, self.colors),
            Tab::Pose => crate::panels::pose::show(ui, self.app),
            Tab::Properties => properties::show(ui, self.app),
            Tab::History => crate::panels::history::show(ui, self.app),
            Tab::Assets => assets::show(ui, self.app),
            // 編集・ポーズのモードでは、ブラシと色の欄を暗くして押せない（理由はツールチップ）
            Tab::SubTools | Tab::ToolProperties | Tab::BrushSize | Tab::ColorSets
                if !self.app.mode.paints() =>
            {
                let rect = ui.max_rect();
                w::enabled_scope(ui, ("mode.dim", tab.key()), false, |ui| match tab {
                    Tab::SubTools => crate::panels::subtools::show(ui, self.app),
                    Tab::ToolProperties => crate::panels::tool_props::show(ui, self.app),
                    Tab::BrushSize => crate::panels::tool_props::show_sizes(ui, self.app),
                    _ => crate::panels::colorsets::show(ui, self.app),
                });
                crate::mode::dimmed_reason(ui, self.app, rect, tab.key());
            }
            Tab::SubTools => crate::panels::subtools::show(ui, self.app),
            Tab::ToolProperties => crate::panels::tool_props::show(ui, self.app),
            Tab::BrushSize => crate::panels::tool_props::show_sizes(ui, self.app),
            Tab::Material => crate::panels::material::show(ui, self.app),
            Tab::ColorSets => crate::panels::colorsets::show(ui, self.app),
            Tab::Log => crate::panels::log::show(ui, self.app),
            Tab::Actions => crate::panels::actions::show(ui, self.app),
        }
    }

    fn on_tab_button(&mut self, tab: &mut Tab, response: &egui::Response) {
        self.tab_rects.insert(*tab, response.rect);
        self.grabbed |=
            response.is_pointer_button_down_on() || response.dragged() || response.drag_stopped();
        if response.drag_stopped() {
            self.released = Some(*tab);
        }
        if response.secondary_clicked() {
            let at = response
                .interact_pointer_pos()
                .unwrap_or(response.rect.left_bottom());
            self.context = Some((*tab, at));
        }
    }

    fn is_closeable(&self, _tab: &Tab) -> bool {
        false
    }

    fn allowed_in_windows(&self, _tab: &mut Tab) -> bool {
        self.windows_allowed
    }

    fn scroll_bars(&self, _tab: &Tab) -> [bool; 2] {
        [false, false]
    }
}

/// 終わってよいかの問いへの答え（試験がウィンドウを開かずに答える口）。
type CloseAnswer = Box<dyn FnMut(&AppState) -> bool>;

/// アプリ。
pub struct YoluApp {
    pub state: AppState,
    pub dock: DockState<Tab>,
    display: CanvasDisplay,
    thumbs: Thumbnails,
    colors: ColorTextures,
    pen: PenInput,
    /// フレームの間隔の下限（垂直同期を待たない表示のとき。待つ表示と試験では何もしない。`pacing`）。
    pacing: crate::pacing::Pacing,
    /// サイドボタンを押したペンの接触を、egui の部品にも右ボタンとして届ける。
    pen_buttons: crate::pen::ButtonMap,
    view3d: View3dSlot,
    /// 3D ビューの wgpu の描画（wgpu の装置が無ければ None）。
    renderer3d: Option<View3dRenderer>,
    /// 主の wgpu のデバイスの見張り（実際のウィンドウだけ。`watch_gpu`）。GPU を失ったときの流れは `gpu_lost`。
    gpu_watch: Option<crate::gpu_watch::GpuWatch>,
    /// 主の GPU を失った（このあと、戻らずにプロセスを終える。戻るのは終え方を替えた試験だけ）。
    gpu_lost: Option<crate::gpu_watch::Lost>,
    /// GPU を失ったとき、書き置きの書き込み・保存の途中の分を待つ長さの上限（取るときも、終わるときも。試験が短くする）。
    gpu_lost_wait: std::time::Duration,
    /// GPU を失ったとき、保存の途中の分を待つ長さの上限（試験が短くする）。
    gpu_lost_save_wait: std::time::Duration,
    /// GPU を失って終わるときのプロセスの終え方（None は `std::process::exit`。試験が替える）。
    gpu_lost_exit: Option<gpu_lost::Exit>,
    /// 試験用: フレームの中の見張りを読む場所に着いたときに呼ばれる物（`set_frame_probe`）。
    frame_probe: Option<Box<dyn FnMut(FramePoint)>>,
    /// 最後のフレームのドックのタブのボタンの矩形（試験用。ドックのタブは読み上げの名前を持たない）。
    pub tab_rects: HashMap<Tab, Rect>,
    link: LiveLink,
    /// 外からの操作（MCP のクライアント・コマンドライン）を受ける（設定「外からの操作を受ける」が入っている間だけ 127.0.0.1 で待つ）。
    ops: McpServer,
    /// ファイルのウィンドウ・確認のウィンドウを開くか（eframe のウィンドウだけ。試験では開かず、頼みを `state.dialog_request` に残す）。
    dialogs: bool,
    /// 終わると決めた（閉じる頼みを二度聞かない）。
    closing: bool,
    /// OS の終了を待たせる印（`session_end::set_saving`）に最後に伝えた「保存の間か」。ウィンドウが見えている間は `ui`、隠れている間は `logic` が
    /// 保存の結果を受けるたびに合わせる。
    saving_marked: bool,
    /// 試験用: 保存していない変更のまま終わってよいかの問いに、ウィンドウを開かずに答える（ウィンドウを開かない試験が、保存の後に問われるかを見る）。
    close_answer: Option<CloseAnswer>,
    /// OS の枠を外したウィンドウか（Windows の実際のウィンドウだけ true。帯の右端に最小化・最大化・閉じるを置き、ウィンドウの縁で大きさを変える）。
    /// 設定には出さない。試験は `set_custom_frame` で選ぶ。
    custom_frame: bool,
    /// 前のフレームの帯で、egui の押しを持たずに生の押しで動く部品（Live Link の印）の矩形。ウィンドウの縁は、この上の押しを譲る
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
    /// ドックの並びとウィンドウの大きさ・位置を書く場所（設定のフォルダの `layout.json`。設定のフォルダが無い試験は None）。
    layout_path: Option<std::path::PathBuf>,
    /// 最後に書いた（または読んだ）ファイルの中身。変わったときだけ書く。
    /// 利用者が並びを動かしていない（仕切りもタブも別ウィンドウも）間の「自動」: Some なら、右の列の割合を最後に合わせたウィンドウの幅（`default_dock_for` の幅）。
    /// ウィンドウの幅が変わると、並びの形と割合がその幅の既定と同じなら、割合だけを今の幅の既定の値に入れ替える（`fit_default_dock`）。違えば（利用者が動かした）None。
    dock_auto: Option<f32>,
    /// 保存した並びが自動のまま閉じられていた（ファイルの `auto_fit`）。最初のフレームで、割合を比べずに今の幅へ合わせ直す。
    dock_refit: bool,
    layout_saved: String,
    /// 最後に「書くか」を見た時刻（egui の時刻。1 秒おきに見る）。
    layout_checked_at: f64,
    /// 最後に見た、最大化していないウィンドウの大きさと位置（終わるときに書く）。
    window_record: Option<crate::layout::WindowRecord>,
    /// 起動のあと、ウィンドウが画面より大きくないか確かめたか。
    window_checked: bool,
    /// 起動のあと、ウィンドウが画面より大きければ収めるか（実際のウィンドウだけ。試験は `fit_to_screen` で選ぶ）。
    fit_window: bool,
    /// 外へ出したウィンドウ（OS のウィンドウ。中のドック・置き場所）。
    pub detached: crate::detach::Detached,
    /// メインウィンドウで描いた、落としたファイルの行き先（ブラシの一覧・ライブラリの格子）。
    root_drops: crate::detach::DropRects,
    /// メインウィンドウの HWND の値（Windows。別ウィンドウの持ち主にする）。
    #[cfg(windows)]
    main_hwnd: Option<isize>,
    /// 落とした PSD のうち、取り込まなかった数（取り込みの仕事が終わったときの文に、理由として足す。0 なら無い）。
    psd_drop_more: usize,
}

mod detached;
mod gpu_lost;
#[doc(hidden)]
pub use gpu_lost::FramePoint;
/// GPU でエラーが起きて終わるときのプロセスの終了コード（`gpu_lost`）。
pub use gpu_lost::EXIT_CODE as GPU_LOST_EXIT_CODE;

impl YoluApp {
    /// 文脈に配色・書体・アイコンを入れる（ウィンドウを作るときに 1 度）。
    pub fn setup(ctx: &egui::Context) {
        // egui の既定は、Ctrl+-・Ctrl++・Ctrl+0 で画面全体（文字も部品も）の拡大率を変える。アプリのキーはこの組み合わせを
        // キャンバスの拡大・縮小に使うので、同じキーで画面全体まで縮んで戻せなくなる。画面全体の拡大縮小は切る
        ctx.options_mut(|o| o.zoom_with_keyboard = false);
        t::apply(ctx);
        fonts::install(ctx);
        icons::install(ctx);
    }

    /// eframe のウィンドウから作る（Windows ではペンの入力をウィンドウに繋ぐ）。
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
        // 主の GPU を失ったとき・受け手の無い誤りを受ける（wgpu の既定は、失っても黙り、誤りは panic で落とす）
        if let Some(rs) = &cc.wgpu_render_state {
            app.watch_gpu(rs, &cc.egui_ctx);
        }
        app.dialogs = true;
        // ファイルを選ぶウィンドウの始まりの場所: 前に使った場所を、設定のフォルダの `places.conf` から読む（実際のウィンドウだけ）
        app.state.places =
            crate::dialog::places::Places::load(crate::dialog::places::Places::default_path());
        #[cfg(windows)]
        {
            use raw_window_handle::{HasWindowHandle, RawWindowHandle};
            if let Ok(handle) = cc.window_handle() {
                if let RawWindowHandle::Win32(h) = handle.as_raw() {
                    app.main_hwnd = Some(h.hwnd.get());
                }
            }
        }
        // 保存は裏のスレッドで動かす（描ける・見られる。試験の状態は、保存の頼みの中で終える）
        app.state.save.background = true;
        app.fit_window = true;
        // Windows は OS の枠を外している（main.rs）ので、帯と縁は自前
        app.custom_frame = titlebar::CUSTOM_FRAME;
        // 状態の帯の右端に版とビルドを出す（実際のウィンドウだけ。試験の画像がコミットごとに変わらないように）
        app.state.usage.build = Some(crate::usage::build_label());
        if let Some(dir) = crate::crash::directory() {
            app.state.crash = crate::crash::window::Report::load(dir);
        }
        // ブラシの見本のストロークは別のスレッドで描く（取り込んだ大きな筆先でも画面が止まらない）。試験のウィンドウは画面のスレッドで描く
        app.state.brushes.samples.render_in_background(&cc.egui_ctx);
        app.state.brushes.krita.load_in_background();
        // 本物の OS のクリップボード（画像のコピー・貼り付け）に繋ぐ。試験のウィンドウは繋がない
        app.state.clip.use_system();
        // 3D ビューには、まず試しの立方体を出しておく（Live Link のモデルが来たら入れ替わる）
        app.state.view3d.load_demo();
        // 落ちた前の実行があれば復旧のウィンドウを開く（起動の引数のプロジェクトより先に。どちらも開ける）
        app.start_recovery();
        // 関連付け（.ylp のダブルクリック）で起動されたら、そのプロジェクトを開く
        app.open_startup_project(std::env::args_os());
        // 更新: 初めてなら問いを出し、「確かめる」を選んでいれば確かめる（実際のウィンドウだけ。試験は呼ばない）
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

    /// 復旧を始める（設定のフォルダの下の置き場。前の実行が落ちていれば復旧のウィンドウが開く）。始められなければ使わず、理由を状態の帯に出す。
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

    /// 設定のファイル（無ければ保存しない）から言語・書き出しのパディング・メモリの予算・退避を残す数などを決めて作る（`setup` は呼ぶ側で）。
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
        // 垂直同期を待たない表示（設定の既定）では、フレームの間隔に下限をかける。ウィンドウの面の同期は起動のときに決まる
        // （`wgpu_configuration`）ので、起動のとき読んだ設定の値で決める（変えた値は次の起動から）
        app.pacing = crate::pacing::Pacing::new(loaded.vsync);
        // macOS のタブレットの筆圧を読むか（試し。ほかの OS では何も起きない）
        app.pen.set_tablet(loaded.tablet_pressure);
        // Windows のペンを WinTab で読むか（既定は Windows Ink。ほかの OS では何も起きない）
        app.pen
            .set_wintab(loaded.pen_input == crate::settings::PenApi::WinTab);
        // 前のプロセスが残したディスクキャッシュのファイル（電源が落ちたときなど）を、起動の邪魔をしないよう裏で消す
        let cache_folder = loaded.disk_cache_folder();
        let _ = std::thread::Builder::new()
            .name("yolu-cache-sweep".into())
            .spawn(move || yolu_core::tile_cache::remove_stale_files(&cache_folder));
        // 利用者のブラシは設定のフォルダの brushes/（読めないファイルは読み飛ばし、知らせる）
        if let Some(dir) = settings.as_deref().and_then(|p| p.parent()) {
            // ツールの並び（tools.json）も、ここで一緒に読む
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
            app.state.path.presets.attach(dir.join("path_presets"));
        }
        // サムネイルは中身の札でキャッシュのフォルダに覚える（作り直せる写し。設定のファイルが無ければ覚えない）
        app.state
            .library
            .attach_cache(settings.as_deref().and_then(crate::library::cache::dir_for));
        let mut notices: Vec<String> = Vec::new();
        notices.extend(startup_message(lang, &problems));
        // キー・マウス・パイの設定（keymap.json。読めなければ既定のキーで始め、ファイルは残す）
        if let Some(dir) = settings.as_deref().and_then(|p| p.parent()) {
            let state = &mut app.state;
            notices.extend(state.keys.attach(
                dir.join(crate::keyconfig::FILE_NAME),
                &mut state.pie.menus,
                lang,
            ));
            crate::keymap::install(state.keys.map());
        }
        notices.extend(app.state.brush_problem_message());
        notices.extend(app.state.toolset_problem_message());
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
        // アクション（読めなかったファイルは消さずに、起動時の知らせに添える）
        let actions = settings
            .as_deref()
            .and_then(|p| p.parent())
            .and_then(|dir| crate::automation::attach(&mut app.state, dir.join("actions")));
        if let Some(actions) = actions {
            if !startup.is_empty() {
                startup.push_str(" / ");
            }
            startup.push_str(&actions);
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

        // ドックの並びとウィンドウの大きさ・位置は、設定のフォルダの layout.json から戻す（読めない・古い・知らないタブは捨てて既定の並び。
        // 理由は診断のログだけ）
        if let Some(path) = settings.as_deref().and_then(crate::layout::path_for) {
            let layout = crate::layout::load(&path);
            for reason in &layout.problems {
                crate::crash::problem(reason.clone());
            }
            if let Some(mut dock) = layout.dock {
                // 前の版の、アプリの中の浮いたウィンドウは、別ウィンドウにする（メインウィンドウの内側の左上からの位置のまま、メインウィンドウの上へ）
                for float in crate::detach::take_floats(&mut dock, |_| None) {
                    app.detached.open(
                        float.dock,
                        Vec::new(),
                        crate::detach::Place::OverMain {
                            offset: [float.rect.min.x, float.rect.min.y],
                            size: crate::detach::new_window_size(Some(float.rect)),
                        },
                    );
                }
                app.dock = dock;
                // 自動のまま閉じた並びは、最初のフレームの幅に合わせ直す。そうでなければ合わせない（はっきり外す）
                app.dock_auto = layout.auto_fit.then_some(REFERENCE_WIDTH);
                app.dock_refit = layout.auto_fit;
            }
            for record in layout.detached {
                let place = match record.window {
                    Some(window) => crate::detach::Place::Record(window),
                    None => crate::detach::Place::Center {
                        size: crate::detach::DEFAULT_SIZE,
                    },
                };
                app.detached.open(record.dock, record.home, place);
            }
            app.window_record = layout.window;
            if path.exists() && layout.problems.is_empty() {
                app.layout_saved = app.render_layout();
            }
            app.layout_path = Some(path);
        }
        app.settings = settings.map(|path| (path, loaded));
        app
    }

    /// 設定のウィンドウで表示の合成が変わっていれば、キャンバスの表示に入れる（次の合成から効く。保存・書き出しの合成は変わらず CPU）。
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

    /// 設定（言語・書き出しのパディング・メモリの予算・スレッド・合成・棚の場所・退避を残す数・選択範囲の帯）の選択が変わっていれば、設定のファイルに書く。
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
            now.uv_overlap_color = saved.uv_overlap_color;
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

    /// ドックの並びとウィンドウの大きさ・位置を、変わっていれば `layout.json` へ書く（毎フレーム呼び、1 秒おきに見る。区切りを動かしている間も
    /// 1 秒おきに 1 回までなので、書き込みが続かない。タブの見出しをつかんでいる間は並びが変わらない）。ウィンドウの大きさと位置は、最大化していない
    /// 間の値を覚える（最大化したまま終わっても、戻したときの大きさを書く）。書けなくても動作は変えない（診断のログへ。同じ中身では書き直さない）。
    fn persist_layout(&mut self, ctx: &egui::Context) {
        // 起動のウィンドウの置き場所を合わせている間（最初の数フレーム）は、途中の位置を記録・保存しない
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
        // 起動のあと 1 度、ウィンドウが画面より大きければ画面に収める（保存したあとで画面が小さくなったとき）
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
        let now = ctx.input(|i| i.time);
        if now - self.layout_checked_at < 1.0 {
            return;
        }
        self.layout_checked_at = now;
        self.save_layout(false);
    }

    /// 並びのファイルの中身（主のドック・メインウィンドウ・別ウィンドウ）。
    fn render_layout(&self) -> String {
        let window = self
            .window_record
            .and_then(crate::layout::WindowRecord::sanitized);
        // タブの無いウィンドウは書かない（読み直しの検証で、並び全部が既定へ戻る）
        let detached: Vec<crate::layout::DetachedRecord> = self
            .detached
            .windows
            .iter()
            .filter(|w| w.dock.main_surface().num_tabs() > 0)
            .map(|w| crate::layout::DetachedRecord {
                dock: w.dock.clone(),
                window: w.record,
                home: w.home.clone(),
            })
            .collect();
        let auto_fit = self.dock_is_auto(self.dock_refit);
        crate::layout::render_full(&self.dock, window.as_ref(), &[], &detached, auto_fit)
    }

    /// 並びを書く。`force` でなければ、前に書いた中身と同じなら書かない。
    fn save_layout(&mut self, force: bool) {
        if self.layout_path.is_none() {
            return;
        }
        let text = self.render_layout();
        let Some(path) = &self.layout_path else {
            return;
        };
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
            pacing: crate::pacing::Pacing::default(),
            pen_buttons: crate::pen::ButtonMap::default(),
            view3d: View3dSlot::default(),
            renderer3d: None,
            gpu_watch: None,
            gpu_lost: None,
            gpu_lost_wait: gpu_lost::RECOVERY_WAIT,
            gpu_lost_save_wait: gpu_lost::SAVE_WAIT,
            gpu_lost_exit: None,
            frame_probe: None,
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
            dock_auto: Some(REFERENCE_WIDTH),
            dock_refit: false,
            layout_saved: String::new(),
            layout_checked_at: f64::NEG_INFINITY,
            window_record: None,
            window_checked: false,
            fit_window: false,
            detached: crate::detach::Detached::new(),
            root_drops: crate::detach::DropRects::default(),
            #[cfg(windows)]
            main_hwnd: None,
            saving_marked: false,
        }
    }

    /// 起動のあと 1 度、ウィンドウが画面より大きければ画面に収める（実際のウィンドウだけ。保存したあとで画面が小さくなったときのため）。
    pub fn fit_to_screen(mut self, on: bool) -> YoluApp {
        self.fit_window = on;
        self
    }

    /// 試験用: 保存していない変更のまま終わってよいかの問いに、ウィンドウを開かずに答える（問われるたびに呼ぶ。保存の後の状態で問われること・
    /// 問われないことを確かめる）。
    #[doc(hidden)]
    pub fn answer_close_question(&mut self, answer: impl FnMut(&AppState) -> bool + 'static) {
        self.close_answer = Some(Box::new(answer));
    }

    /// 終わると決めたか（保存の途中なら、保存が終わるまで決めない）。
    pub fn is_closing(&self) -> bool {
        self.closing
    }

    /// 試験用: OS の終了を待たせる印に、最後に「保存の間」と伝えたか（Windows の実際のウィンドウでなくても、伝える側の状態を確かめる）。
    #[doc(hidden)]
    pub fn saving_marked(&self) -> bool {
        self.saving_marked
    }

    /// 帯の右端のボタンとウィンドウの縁を自前にするか（Windows の実際のウィンドウは true。試験は Linux でも Windows の帯を描いて確かめる）。
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

    /// Live Link を始める・やめるの頼みと、ファイルのウィンドウの頼みを当てる。
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
        // 復旧の世代を開く頼み（今の変更を捨ててよいか聞いてから。ウィンドウを開かない試験では聞かない）
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
                // 保存していない変更は先に聞く（ウィンドウを開いてから聞くと、作業を捨てる前にウィンドウの設定が無駄になる）
                if self.confirm_discard() {
                    self.state
                        .apply(Action::Project(crate::newproject::NpAction::OpenNew));
                }
            }
            Some(DialogRequest::ProjectModel) => {
                let lang = self.state.lang;
                if let Some(path) = crate::dialog::file(&self.state, Place::Model)
                    .set_title(lang.pick("モデルを選ぶ", "Choose a model"))
                    .add_filter("FBX", &["fbx", "FBX"])
                    .pick_file()
                {
                    self.state.note_file_chosen(Place::Model, &path);
                    self.state
                        .apply(Action::Project(crate::newproject::NpAction::ChooseModel(
                            path,
                        )));
                }
            }
            Some(DialogRequest::Open) => {
                let lang = self.state.lang;
                if self.confirm_discard() {
                    if let Some(path) = crate::dialog::file(&self.state, Place::Project)
                        .set_title(lang.pick("プロジェクトを開く", "Open Project"))
                        .add_filter(
                            lang.pick("YoluPainter プロジェクト", "YoluPainter Project"),
                            &["ylp"],
                        )
                        .pick_file()
                    {
                        self.state.note_file_chosen(Place::Project, &path);
                        self.state.apply(Action::OpenProject(path));
                    }
                }
            }
            Some(DialogRequest::SaveAs) => {
                let lang = self.state.lang;
                let name = format!("{}.ylp", self.state.project_name);
                let dialog = crate::dialog::file_for_save(&self.state, Place::Project)
                    .set_title(lang.pick("別名で保存", "Save As"))
                    .add_filter(
                        lang.pick("YoluPainter プロジェクト", "YoluPainter Project"),
                        &["ylp"],
                    )
                    .set_file_name(name);
                if let Some(path) = dialog.save_file() {
                    self.state.note_file_chosen(Place::Project, &path);
                    self.state.apply(Action::SaveProjectAs(path));
                }
            }
            Some(DialogRequest::OpenModel) => {
                let lang = self.state.lang;
                if let Some(path) = crate::dialog::file(&self.state, Place::Model)
                    .set_title(lang.pick("3D ビューに FBX を開く", "Open FBX in the 3D View"))
                    .add_filter("FBX", &["fbx", "FBX"])
                    .pick_file()
                {
                    self.state.note_file_chosen(Place::Model, &path);
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
            Some(DialogRequest::ExportDestination) => {
                crate::export::window::run_dialog(&mut self.state)
            }
            Some(DialogRequest::PrefsLibraryFolder) => {
                let lang = self.state.lang;
                let mut dialog = crate::dialog::file(&self.state, Place::Settings)
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
                    self.state.note_folder_chosen(Place::Settings, &dir);
                    self.state
                        .apply(Action::Prefs(crate::prefs::PrefsAction::Set(
                            crate::prefs::Pref::LibraryFolder(Some(dir)),
                        )));
                }
            }
            Some(DialogRequest::ToolsetReset) => {
                let lang = self.state.lang;
                let yes = crate::dialog::message()
                    .set_title("YoluPainter")
                    .set_description(lang.pick(
                        "ツールの並びを最初の並びに戻しますか？（ツールとグループの名前・並びは取り消せません。ブラシのファイルは残ります）",
                        "Reset the tool layout? (Tool and group names and order cannot be restored. Brush files stay.)",
                    ))
                    .set_buttons(rfd::MessageButtons::YesNo)
                    .set_level(rfd::MessageLevel::Warning)
                    .show()
                    == rfd::MessageDialogResult::Yes;
                if yes {
                    self.state
                        .apply(Action::Tools(crate::toolset::ToolsetAction::Reset));
                }
            }
            Some(DialogRequest::BrushFileDelete) => {
                let lang = self.state.lang;
                let Some(key) = self.state.toolset.catalog.pending_delete.take() else {
                    return;
                };
                let name = self
                    .state
                    .brushes
                    .lib
                    .entry(key)
                    .map(|e| e.name.clone())
                    .unwrap_or_default();
                let yes = crate::dialog::message()
                    .set_title("YoluPainter")
                    .set_description(lang.pick(
                        format!("ブラシのファイル「{name}」を消しますか？（取り消せません）"),
                        format!("Delete the brush file \"{name}\"? (This cannot be undone.)"),
                    ))
                    .set_buttons(rfd::MessageButtons::YesNo)
                    .set_level(rfd::MessageLevel::Warning)
                    .show()
                    == rfd::MessageDialogResult::Yes;
                if yes {
                    self.state
                        .apply(Action::Brush(crate::brushes::BrushAction::DeleteFile(key)));
                }
            }
            Some(DialogRequest::PrefsCacheFolder) => {
                let lang = self.state.lang;
                let mut dialog = crate::dialog::file(&self.state, Place::Settings)
                    .set_title(lang.pick("キャッシュの場所", "Cache folder"));
                let current = self.state.prefs.settings.disk_cache_folder();
                if current.is_dir() {
                    dialog = dialog.set_directory(current);
                }
                if let Some(dir) = dialog.pick_folder() {
                    self.state.note_folder_chosen(Place::Settings, &dir);
                    self.state
                        .apply(Action::Prefs(crate::prefs::PrefsAction::Set(
                            crate::prefs::Pref::DiskCacheFolder(Some(dir)),
                        )));
                }
            }
            Some(DialogRequest::PsdImport(target)) => {
                let lang = self.state.lang;
                // 今の文書を替えるときは、保存していない変更を捨ててよいか聞く
                if target == crate::psd::PsdTarget::NewSet || self.confirm_discard() {
                    if let Some(path) = crate::dialog::file(&self.state, Place::PsdImport)
                        .set_title(lang.pick("PSD を読み込む", "Import PSD"))
                        .add_filter("PSD", &["psd", "PSD"])
                        .pick_file()
                    {
                        self.state.note_file_chosen(Place::PsdImport, &path);
                        self.state
                            .apply(Action::Psd(crate::psd::PsdAction::Import { path, target }));
                    }
                }
            }
            Some(DialogRequest::PsdExport) => {
                let lang = self.state.lang;
                let name = crate::psd::default_export_name(&self.state);
                let dialog = crate::dialog::file(&self.state, Place::PsdExport)
                    .set_title(lang.pick("PSD に書き出す", "Export PSD"))
                    .add_filter("PSD", &["psd"])
                    .set_file_name(name);
                if let Some(path) = dialog.save_file() {
                    self.state.note_file_chosen(Place::PsdExport, &path);
                    self.state
                        .apply(Action::Psd(crate::psd::PsdAction::Export(path)));
                }
            }
            Some(DialogRequest::DistributeSave) => crate::distribute::run_dialog(&mut self.state),
            Some(DialogRequest::TextFont) => {
                let lang = self.state.lang;
                if let Some(path) = crate::dialog::file(&self.state, Place::Font)
                    .set_title(lang.pick("フォントのファイルを開く", "Open a Font File"))
                    .add_filter(
                        lang.pick("フォント", "Fonts"),
                        &["ttf", "otf", "ttc", "TTF", "OTF", "TTC"],
                    )
                    .pick_file()
                {
                    self.state.note_file_chosen(Place::Font, &path);
                    self.state
                        .apply(Action::Text(crate::textlayer::TextAction::FontFile(path)));
                }
            }
            Some(DialogRequest::KeymapExport) => {
                let lang = self.state.lang;
                if let Some(path) = crate::dialog::file(&self.state, Place::Keys)
                    .set_title(lang.pick("キーの設定を書き出す", "Export Key Settings"))
                    .set_file_name(crate::keyconfig::FILE_NAME)
                    .add_filter("JSON", &["json", "JSON"])
                    .save_file()
                {
                    self.state.note_file_chosen(Place::Keys, &path);
                    self.state.keys_export(&path);
                }
            }
            Some(DialogRequest::KeymapImport) => {
                let lang = self.state.lang;
                if let Some(path) = crate::dialog::file(&self.state, Place::Keys)
                    .set_title(lang.pick("キーの設定を読み込む", "Import Key Settings"))
                    .add_filter("JSON", &["json", "JSON"])
                    .pick_file()
                {
                    self.state.note_file_chosen(Place::Keys, &path);
                    self.state.keys_import(&path);
                }
            }
            Some(DialogRequest::OpenStencil) => {
                let lang = self.state.lang;
                if let Some(path) = crate::dialog::file(&self.state, Place::ImageImport)
                    .set_title(lang.pick("ステンシルの画像を開く", "Open a stencil image"))
                    .add_filter("PNG", &["png", "PNG"])
                    .pick_file()
                {
                    self.state.note_file_chosen(Place::ImageImport, &path);
                    self.state
                        .apply(Action::Stencil(crate::stencil::StencilOp::Load(path)));
                }
            }
            Some(DialogRequest::FillImage) => {
                let lang = self.state.lang;
                if let Some(path) = crate::dialog::file(&self.state, Place::ImageImport)
                    .set_title(lang.pick(
                        "画像をアセットへ取り込む",
                        "Add an image to the project's assets",
                    ))
                    .add_filter("PNG", &["png", "PNG"])
                    .pick_file()
                {
                    self.state.note_file_chosen(Place::ImageImport, &path);
                    self.state
                        .apply(Action::Fill(crate::fillfx::FillOp::ImportImage(path)));
                }
            }
            Some(DialogRequest::NewFillImage(mode)) => {
                let lang = self.state.lang;
                // 選ばずに閉じたら何も作らない（Undo の段も増やさない）
                if let Some(path) = crate::dialog::file(&self.state, Place::ImageImport)
                    .set_title(lang.pick("画像で塗りつぶしを作る", "Create a fill from an image"))
                    .add_filter("PNG", &["png", "PNG"])
                    .pick_file()
                {
                    self.state.note_file_chosen(Place::ImageImport, &path);
                    self.state
                        .apply(Action::LayerMenu(crate::layermenu::Op::FillImageFile {
                            path,
                            mode,
                        }));
                }
            }
            Some(DialogRequest::ImportBrushes) => {
                let lang = self.state.lang;
                if let Some(paths) = crate::dialog::file(&self.state, Place::Brush)
                    .set_title(lang.pick("ブラシを取り込む", "Import Brushes"))
                    .add_filter(
                        lang.pick("ブラシのファイル", "Brush files"),
                        &yolu_io::brushes::FileKind::EXTENSIONS,
                    )
                    .pick_files()
                {
                    if let Some(first) = paths.first() {
                        self.state.note_file_chosen(Place::Brush, first);
                    }
                    self.state
                        .apply(Action::Brush(crate::brushes::BrushAction::Import(paths)));
                }
            }
            Some(DialogRequest::ClipStudioFolder) => {
                let lang = self.state.lang;
                let mut dialog =
                    crate::dialog::file(&self.state, Place::Settings).set_title(lang.pick(
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
                    self.state.note_folder_chosen(Place::Settings, &dir);
                    self.state.apply(Action::Brush(
                        crate::brushes::BrushAction::ClipStudioFolder(dir),
                    ));
                }
            }
            None => {}
        }
    }

    /// 保存していない変更があっても終わってよいか（ウィンドウを開かない試験では聞かない）。保存の途中には聞かない（保存が終わってから、その後の
    /// 状態で聞く）。
    fn confirm_close(&mut self) -> bool {
        // 更新のために終わるときは、保存するか捨てるかを更新のウィンドウで選び済み。GPU を失って終わるときは、復旧の書き置きに任せる
        if self.state.update.is_quitting() || self.gpu_lost.is_some() {
            return true;
        }
        // 閉じると取り消される仕事（利用者が結果を待っている書き出しなど）も、保存していない変更と一緒に知らせる
        let jobs = crate::windows::close_jobs(&self.state);
        if !self.state.modified && jobs.is_empty() {
            return true;
        }
        // 試験が、ウィンドウを開かずに答える口
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

    /// 保存していない変更を捨ててよいか（ウィンドウを開かない試験では、聞かずに捨てる）。保存の途中は、保存の結果が出るまで、頼む前の印のまま
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

    /// ウィンドウに落としたファイル（.ylp なら開く。.psd なら新しいテクスチャセットとして取り込む。ブラシのファイル（ABR・GBR・GIH・VBR・PAT）なら
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

    /// 落とした .psd を、「ファイル → インポート → PSD を新しいテクスチャセットに」と同じ取り込み（読み込み → 取り込みの確認のウィンドウ）へ回す。
    /// 取り込むのは最初の 1 つだけ（取り込みの確認のウィンドウは 1 つずつ。ほかは取り込まず、数を知らせる）。描いている最中は、取り込み側が断る。
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

    /// GPU のテクスチャの辺の上限（試験用。wgpu が無ければ None）。
    pub fn view3d_texture_limit(&self) -> Option<u32> {
        self.renderer3d.as_ref().map(|r| r.max_texture_dimension())
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

    /// 3D が次のフレームも求めているか（接線を作っている最中など。ウィンドウの描き直しの要求と同じ）。
    pub fn view3d_wants_repaint(&self) -> bool {
        self.renderer3d.as_ref().is_some_and(|r| r.wants_repaint())
    }

    /// 試験用: 塗った絵のバイトの予算を小さくして、縮めの道を通す。
    pub fn view3d_set_paint_budget(&mut self, bytes: u64) {
        if let Some(r) = &mut self.renderer3d {
            r.set_paint_budget(bytes);
        }
    }

    /// 塗った絵のサンプラーが今使っている異方性の上限（試験・計測用。wgpu が無ければ None）。
    pub fn view3d_paint_anisotropy(&self) -> Option<u16> {
        self.renderer3d.as_ref().map(|r| r.paint_anisotropy())
    }

    /// 試験用: 塗った絵のサンプラーの異方性の上限を変える（1 で等方。GPU が持たなければ 1 のまま）。
    pub fn view3d_set_paint_anisotropy(&mut self, wanted: u16) {
        if let Some(r) = &mut self.renderer3d {
            r.set_paint_anisotropy(wanted);
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

    /// 試験用: 塗った絵の重みの絵の 1 段の中身（1 テクセル 1 バイト、行は下から。大きさつき。塗り広げていなければ None）。
    pub fn view3d_read_paint_weight_level(&self, level: u32) -> Option<(Vec<u8>, [u32; 2])> {
        self.renderer3d.as_ref()?.read_paint_weight_level(level)
    }

    /// 試験用: ほかのテクスチャセット（マテリアルの番号）の絵の重みの絵の 1 段の中身（絵を持っていなければ None）。
    pub fn view3d_read_other_weight_level(
        &self,
        material: i32,
        level: u32,
    ) -> Option<(Vec<u8>, [u32; 2])> {
        self.renderer3d
            .as_ref()?
            .read_other_weight_level(material, level)
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

    /// 試験・計測用: 3D ビューの表示の写しを UV の外へ塗り広げる幅（表示のテクセル。0 は塗り広げない前の仕事）。
    pub fn view3d_set_display_padding(&mut self, texels: u32) {
        if let Some(r) = &mut self.renderer3d {
            r.set_display_padding(texels);
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

    /// 試験用: メインウィンドウのペンの受け口の、ウィンドウをアプリの側で動かす手を差し替える（None は手が無い受け口。Windows 以外と同じ）。
    #[doc(hidden)]
    pub fn set_pen_mover(&mut self, mover: Option<std::sync::Arc<dyn crate::pen::WindowMover>>) {
        self.pen = self.pen.clone().with_mover(mover);
    }

    /// egui が受けるポインタの入力のうち、サイドボタンを押したペンの接触を右ボタンに直す（`eframe::App::raw_input_hook` が毎フレーム
    /// 呼ぶ。winit はペンを左ボタンの押しにしか変えないので、これが無いと、ペンではどの部品の右クリックのメニューも開かない）。
    pub fn remap_pen_buttons(&mut self, events: &mut [egui::Event]) {
        self.pen_buttons.remap(&self.pen.peek(), events);
    }

    /// フレームの初め（egui のパスの前。`raw_input_hook` が呼ぶ）に、垂直同期を待たない表示のときだけ、前のフレームの始まりからの間が下限より短ければ
    /// 残りを眠る（`pacing`）。入力のあるフレームは 1/240 秒、無いフレーム（描き直しの頼みだけ）はウィンドウのあるモニターのリフレッシュレートの逆数
    /// （60〜240 Hz。読めなければ 1/120 秒。入力の見方は `frame_has_input`、リフレッシュレートの読み方は `read_refresh_rate`）。
    /// 眠った分だけ、このフレームの時刻（`raw_input.time`）を進める。
    pub fn pace_frame(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        if self.pacing.waits_for_vsync() {
            return;
        }
        // 最小化・非表示の間は、eframe が自分で間隔を空ける（`logic` だけを回す。見えている別ウィンドウがあるときは、主が隠れていてもパスは回る）。
        // 覆われているだけのとき（`visible()` は偽）は `logic` だけが回るが、画面に出さないので、垂直同期を待たない設定で増える回りは無い
        let hidden = raw_input
            .viewports
            .get(&raw_input.viewport_id)
            .is_some_and(|info| info.visible() == Some(false));
        if hidden && !self.any_detached_visible(ctx) {
            return;
        }
        if self.pacing.refresh_due(std::time::Instant::now()) {
            self.read_refresh_rate();
        }
        let input = self.frame_has_input(raw_input);
        let slept = self.pacing.wait(input);
        if let Some(time) = raw_input.time.as_mut() {
            *time += slept.as_secs_f64();
        }
    }

    /// ウィンドウのあるモニターのリフレッシュレートを読んで、入力の無いフレームの下限に入れる（1 秒に 1 度まで。`pace_frame` が呼ぶ）。
    /// Windows だけ読む（主のウィンドウと別ウィンドウのあるモニターのうち、いちばん速いもの。`pacing::refresh`）。eframe・winit はウィンドウの
    /// モニターのリフレッシュレートを渡さず、macOS・Linux は安く読める道が無いので、読まずに 1/120 秒のまま（`set_monitor_refresh_hz` で入れた値は消さない）。
    fn read_refresh_rate(&mut self) {
        #[cfg(windows)]
        {
            let hwnds: Vec<isize> = self
                .main_hwnd
                .into_iter()
                .chain(self.detached.windows.iter().filter_map(|w| w.hwnd))
                .collect();
            if !hwnds.is_empty() {
                self.pacing
                    .set_refresh_rate(crate::pacing::refresh::fastest_refresh_hz(&hwnds));
            }
        }
    }

    /// 入力の無いフレームの下限に使うモニターのリフレッシュレートを入れる（Hz。読めない = `None` は 1/120 秒。試験用。Windows の実際のウィンドウは
    /// `read_refresh_rate` が 1 秒ごとに読んで入れ直す）。
    pub fn set_monitor_refresh_hz(&mut self, refresh_hz: Option<f64>) {
        self.pacing.set_refresh_rate(refresh_hz);
    }

    /// 最後にフレームの間隔として選んだ下限（待つ形・まだフレームが無いときは `None`）。試験が、入力のあるフレームに 1/240 秒、
    /// 無いフレームにモニターのリフレッシュレートの逆数を選んだことを見る。
    pub fn frame_interval(&self) -> Option<std::time::Duration> {
        self.pacing.last_interval()
    }

    /// このフレームの初めに見える入力があるか（フレームの間隔の下限を、入力のあるフレームの 1/240 秒にするか）。見るのは、主のウィンドウの egui の事象
    /// （`raw_input.events`）、主と別ウィンドウのペンの待ち行列、前のフレームに別ウィンドウのパスが見た入力の印（別ウィンドウの egui の事象は、主のフレームの
    /// 初めには見えない。印は取り出して消す）。
    pub fn frame_has_input(&mut self, raw_input: &egui::RawInput) -> bool {
        let detached_marked = self.pacing.take_detached_input();
        detached_marked
            || !raw_input.events.is_empty()
            || !self.pen.peek().is_empty()
            || self
                .detached
                .windows
                .iter()
                .any(|w| !w.pen.peek().is_empty())
    }

    /// フレームの間隔に下限をかけているか（垂直同期を待たない表示。起動のとき読んだ設定の「垂直同期」が切のとき）。
    pub fn paces_frames(&self) -> bool {
        !self.pacing.waits_for_vsync()
    }

    /// フレームの間隔の下限を、待つ形（何もしない）か待たない形（下限をかける）に替える（試験が `raw_input_hook` の眠りを確かめるのに使う）。
    pub fn set_frame_pacing(&mut self, waits_for_vsync: bool) {
        self.pacing = crate::pacing::Pacing::new(waits_for_vsync);
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

    /// 3D ビューの中身を描く別ウィンドウの口を渡す。
    pub fn set_view3d_host(&mut self, host: Box<dyn View3dHost>) {
        self.view3d.set_host(host);
    }

    /// 1 フレーム（eframe と試験の両方がここを呼ぶ）。フレームの中で `message` に書かれた文（非同期の終わりなど、`apply` の外の書き込みも）は、
    /// 前と同じ文でも新しい知らせとして出る。
    pub fn frame(&mut self, ui: &mut Ui) {
        self.fit_default_dock(ui.ctx());
        let prior = self.state.message_begin();
        // レイヤーの行のクリックなどで選んでいるレイヤーが替わっていたら、別のレイヤーの定規の選びを外す
        self.state.drop_foreign_ruler_selection();
        self.frame_body(ui);
        self.state.message_end(prior);
        self.finish_message(ui.ctx());
    }

    /// 並びが自動の間（`dock_auto`）、毎フレーム、並びが最後に合わせた幅の既定のままか確かめ（違えば利用者が動かしたので自動をやめる）、ウィンドウの幅が
    /// 変わっていれば、右の列の割合を今の幅の既定の値に入れ替える。並びの形は分け目の種類と、組ごとのタブ（既定の並びに無いナビゲーター・アクション・ポーズは
    /// 数えない）、割合は 1e-4 以内。木は作り直さない（選んでいるタブ・開いたナビゲーターを保つ）。別ウィンドウが 1 つでもあれば自動をやめる。
    /// 保存した並びが自動のまま閉じられていたとき（`dock_refit`）は、形だけを見て、最初のフレームの幅へ割合を合わせ直す。
    fn fit_default_dock(&mut self, ctx: &egui::Context) {
        let refit = std::mem::take(&mut self.dock_refit);
        if !self.dock_is_auto(refit) {
            self.dock_auto = None;
            return;
        }
        let Some(last) = self.dock_auto else {
            return;
        };
        let width = ctx.content_rect().width();
        if !refit && (width - last).abs() <= DOCK_FIT_EPSILON_WIDTH {
            return;
        }
        let now = default_dock_for(width);
        for (node, new) in self
            .dock
            .main_surface_mut()
            .iter_mut()
            .zip(now.main_surface().iter())
        {
            if let (
                Node::Horizontal(split) | Node::Vertical(split),
                Node::Horizontal(want) | Node::Vertical(want),
            ) = (node, new)
            {
                split.fraction = want.fraction;
            }
        }
        self.dock_auto = Some(width);
    }

    /// 並びが自動のままか: 自動の印があり、別ウィンドウが無く、並びが最後に合わせた幅の既定と同じ形（`shape_only` なら割合は見ない）。
    fn dock_is_auto(&self, shape_only: bool) -> bool {
        let Some(last) = self.dock_auto else {
            return false;
        };
        if !self.detached.windows.is_empty() || self.dock.iter_surfaces().count() != 1 {
            return false;
        }
        let reference = default_dock_for(last);
        same_split_shape(
            self.dock.main_surface(),
            reference.main_surface(),
            !shape_only,
        )
    }

    /// フレームの終わりの `message`: 失敗・断り・警告を記録へ（同じ文なら何もしない）。新しい知らせがあれば、すぐ出すための描き直しを頼む。
    fn finish_message(&mut self, ctx: &egui::Context) {
        self.state.record_message();
        if self.state.toast.is_pending() {
            ctx.request_repaint();
        }
    }

    /// 離した後の残りを塗っている 3D のストローク（確定待ち）は、次の操作が来たら、その操作を通す前に、残りを塗って確定する。利用者は離した
    /// 時点で描き終えたと思っているので、断らず・捨てずに、操作をそのまま通すため。操作は、ポインタを押す・キーを押す・ホイール・文字の入力・
    /// 貼り付けなど・ファイルの落とし・ペンが触れる（動くだけ・離すだけでは確定しない）。ここは全部の部品より先に呼ぶ。パネルの部品・メニュー・
    /// ショートカットの全部が、描いている最中の判定（`is_stroking`）を見る前に済む。
    fn settle_before_input(&mut self, ctx: &egui::Context, pen: &[PenSample]) {
        if !self.state.view3d.input.released {
            return;
        }
        let acts = ctx.input(|i| {
            !i.raw.dropped_files.is_empty()
                || i.events.iter().any(|e| {
                    matches!(
                        e,
                        egui::Event::PointerButton { pressed: true, .. }
                            | egui::Event::Key { pressed: true, .. }
                            | egui::Event::MouseWheel { .. }
                            | egui::Event::Text(_)
                            | egui::Event::Paste(_)
                            | egui::Event::Copy
                            | egui::Event::Cut
                            | egui::Event::Touch {
                                phase: egui::TouchPhase::Start,
                                ..
                            }
                    )
                })
        });
        if acts || pen.iter().any(|s| s.contact) {
            crate::view3d::input::settle(&mut self.state);
        }
    }

    fn frame_body(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        // このアプリのキーの割り当て（ショートカットの設定）を、このスレッドで効かせる
        crate::keymap::install(self.state.keys.map());
        self.poll_gpu_watch(&ctx);
        self.state.ui.popup_was_open = self.state.popup.is_some();
        crate::region::bucket::poll(&mut self.state, &ctx);
        // 設定の「タブレットの筆圧（試し）」（設定のウィンドウで切り替えた値を、ペンの受け口へ。別ウィンドウの受け口は同じ札を共有する）
        self.pen
            .set_tablet(self.state.prefs.settings.tablet_pressure);
        // 設定の「ペンの入力」（Windows の WinTab。別ウィンドウは、それぞれのフレームで合わせる）。開けなかったときは Windows Ink のままで、理由を 1 度だけ知らせる
        self.pen
            .set_wintab(self.state.prefs.settings.pen_input == crate::settings::PenApi::WinTab);
        if let Some(why) = self.pen.take_wintab_notice() {
            self.state.wintab_unavailable(why);
        }
        // 補った離し（OS に押しを奪われて、離しの点を補った）の印も一緒に取る。2D と 3D の入力が、本物の離しと分けて扱う
        let (mut pen, lost) = self.pen.drain_with_lost();
        self.state.pen_lost = lost;
        self.settle_before_input(&ctx, &pen);
        // ウィンドウの縁（自前の枠だけ）: 押したら大きさを変える頼みを送る。描いている最中・ペンが触れている最中（キャンバスと 3D ビューが
        // ペンの押しとして扱うのと同じ `contact`。筆圧は触れていなくても 1 のペンも、触れた直後は 0 のペンもある）は受けない
        let edge = self.custom_frame.then(|| {
            titlebar::edges(
                &ctx,
                self.state.is_stroking() || pen.iter().any(|s| s.contact),
                &self.bar_press_rects,
            )
        });
        // 筆圧の調整（設定の「ペン」）: 調整を通す前の筆圧を集め、そのあとで全体の調整（設定）を通してから、キャンバスと 3D ビューへ渡す
        self.state.pressure_observe(ctx.pixels_per_point(), &pen);
        // 描く枠が出ているときだけ（描いている間じゅう毎フレーム、全イベントの写しを作らない）
        if self.state.pressure.open() && !self.pen.is_hooked() {
            ctx.input(|i| self.state.pressure_observe_touch(&i.events));
        }
        for sample in &mut pen {
            sample.pressure = self.state.adjust_pressure(sample.pressure);
        }
        // 別ウィンドウで開いたポップアップは、メインウィンドウを押したら閉じる
        detached::close_foreign_popup(&ctx, &mut self.state);
        // キーの割り当てとステンシルのキーの押しは、フォーカスのあるウィンドウの入力で決める（別ウィンドウにフォーカスがあれば、そのウィンドウのパスが決める。
        // フォーカスの無いウィンドウのパスが見ると、クリップボードのキーの「前のフレームの修飾」を押していない修飾で上書きする）
        if !self.detached_focused(&ctx) {
            shell::handle_shortcuts(&ctx, &mut self.state);
            crate::stencil::update_keys(&ctx, &mut self.state);
        }
        // 落としたファイル。ブラシの一覧とライブラリの格子の範囲は、前のフレームにメインウィンドウで描いたときだけ入る（棚・チャンネルのタブを
        // 開いている間や、欄を別ウィンドウへ出した後に、前の位置へ落とした PNG を取り込まない。別ウィンドウへ落とした分は、そのウィンドウのパスが受ける）
        self.state.brushes.ui.list_rect = self.root_drops.brush_list;
        self.open_dropped(&ctx);
        self.state.brushes.ui.list_rect = None;
        self.state.library.grid_rect = self.root_drops.library_grid;
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
        // 更新の確かめ・ダウンロードの終わり（準備のウィンドウは、描いている最中は開かない）
        self.state.poll_update();
        self.state.poll_clipboard();
        // OS のフォントの一覧ができたら受け、開いた文書のテキストレイヤーのフォントを確かめる
        self.state.text_poll();
        // 文字の色の元が描画色なら、描画色が変わったとき、選んでいるテキストレイヤーの色も変える（円のドラッグは 1 回の取り消し）
        let pointer_down = self.any_pointer_down(&ctx);
        self.state.text_follow_paint_color(pointer_down);
        // 復旧: 書き置きの結果を受け、書く頃なら頼む。フォーカスを失ったら、時間を待たずに書く
        // （メインウィンドウから別ウィンドウへフォーカスが移っても、アプリはフォーカスを失っていない）
        let focused = if self.detached_focused(&ctx) {
            Some(true)
        } else {
            ctx.input(|i| i.viewport().focused)
        };
        if self.was_focused == Some(true) && focused == Some(false) {
            self.state.recovery_request_flush();
        }
        self.was_focused = focused.or(self.was_focused);
        if let Some(wait) = self.state.recovery_tick() {
            ctx.request_repaint_after(wait);
        }
        // 3D ビューで描くマテリアル・隠すマテリアルを今のテクスチャセットに合わせる（ストロークが終わった後のフレームでも）
        self.state.sync_view3d();
        // 重なった UV の図（今のモデル・セット・大きさで数え直す。表示と塗りの知らせ）
        self.state.poll_uv_overlap();
        // ポーズ: 読み終わった FBX を入れる（入れたら 3D ビューのタブを前へ）
        if crate::view3d::pose::frame(&mut self.state, &ctx) {
            self.bring_forward(Tab::View3d);
        }
        self.ensure_pose_tab();
        if self.state.reset_layout {
            self.dock = default_dock_for(ctx.content_rect().width());
            self.dock_auto = Some(ctx.content_rect().width());
            self.dock_refit = false;
            self.detached.clear();
            self.state.reset_layout = false;
        }
        self.state.ui.panels = self.detached.index(&self.dock);

        // 状態の帯の右端のメモリ（実際のウィンドウだけ。1.5 秒おきに測り、止まっていても同じ間隔で描き直す）
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
        // 描いている間は部品の見た目を描き始める前のまま保つ（灰色に替えて点滅させない。押せないことは変えない）
        w::begin_stroke_frame(&ctx, self.state.holds_panel_look());
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
                // 自前の枠: 右端の 3 つのボタンの左までが帯の中身。何も無い所は、ウィンドウを動かす・最大化する部品（メニューの見出しなどより先に作る）
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
                // 右端: プロジェクトの名前と保存の状態。その左に Live Link の入口（Unity の印）。名前は、メニューの見出しの右からウィンドウの右の縁までの
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
        titlebar::send_drag_commands(&ctx, frame_commands, &self.pen);
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
                w::enabled_scope(ui, "yolu.options.scope", !self.state.is_stroking(), |ui| {
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
        // 描く前のタブの組（egui_dock がタブを浮いたウィンドウへ動かしたとき、戻る先にする）
        let mates = crate::detach::leaf_mates(&self.dock);
        let pass = egui::CentralPanel::default()
            .frame(Frame::NONE.fill(t::WINDOW_BG))
            .show(ui, |ui| {
                let style = dock_style(ui.style());
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
                    released: None,
                    context: None,
                    windows_allowed: true,
                };
                dock_area(&mut self.dock, Id::new("yolu.dock"), style).show_inside(ui, &mut tabs);
                self.tab_rects = tabs.tab_rects;
                detached::DockPass {
                    grabbed: tabs.grabbed,
                    released: tabs.released,
                    context: tabs.context,
                }
            })
            .inner;
        // （写すだけ。別ウィンドウが無ければ、このフレームに描いた矩形がそのまま残る）
        self.root_drops = crate::detach::DropRects {
            brush_list: self.state.brushes.ui.list_rect,
            library_grid: self.state.library.grid_rect,
        };
        // タブの出し入れ（メインウィンドウの外で離したタブ・右クリック・egui_dock が作った浮いたウィンドウ）と、別ウィンドウ。当てるのは全部のウィンドウを描いた後
        let mut events = Vec::new();
        detached::tab_events(
            &ctx,
            &mut self.state,
            &self.dock,
            egui::ViewportId::ROOT,
            ctx.content_rect(),
            &pass,
            &mut events,
        );
        let floats = crate::detach::take_floats(&mut self.dock, |i| {
            ctx.memory(|m| m.area_rect(crate::detach::float_area_id(i)))
        });
        if !floats.is_empty() {
            events.push(detached::floats_event(&ctx, floats, mates));
        }
        // ドックのあとに描くウィンドウ（別ウィンドウも）は、描き始めた・終わった今の状態から
        w::update_stroke_hold(&ctx, self.state.holds_panel_look());
        let grabbed_outside = self.show_detached(&ctx, &mut events);
        self.state.ui.dock_grab = [pass.grabbed || grabbed_outside, self.state.ui.dock_grab[0]];
        self.apply_dock_events(&ctx, events);
        detached::refuse_foreign_drop(&ctx);
        // ツールの列・グループのタブ・ブラシの行をまたぐドラッグは、全部を描いたあとに落とす先へ当てる
        crate::toolset::ui::end_frame(&ctx, &mut self.state);
        // 3D ビューのタブが見えているか（次のフレームのキー入力・メニューの取り消しの行き先が読む）
        self.state.view3d.visible = self.view3d.content_rect().is_some();
        self.state.ui.canvas_visible = std::mem::take(&mut self.state.ui.canvas_drawn);
        // 隠れたビューにポインタが乗っている印は残さない
        if !self.state.view3d.visible {
            self.state
                .rulers
                .note_pointer(crate::rulers::Place::View3d, false);
        }
        if !self.state.ui.canvas_visible {
            self.state
                .rulers
                .note_pointer(crate::rulers::Place::Canvas, false);
        }
        // 隠れたビューは、ペンが離れたのを受け取れない（タブの見出しをつかんで動かしているあいだなど）。ペンの押しの印と、ペンが回し・
        // パン・拡縮していた途中を、見えるようになるまで持ち越さない（印が残ると、ペンの押しとみなしてマウスの押しを使わなくなる）
        if !self.state.ui.canvas_visible {
            self.state.drafting_cancel_canvas();
            self.state.drafting.pen_down = None;
            self.state.canvas.pen_press = None;
            crate::canvas::nav::cancel(&mut self.state);
        }
        if !self.state.view3d.visible {
            self.state.view3d.input.drop_presses();
            // 3D ビューで引いていた選択の形と、打っていた多角形の点も、離したのを受け取れないので何も選ばずに捨てる
            self.state.sel.view3d.cancel();
            // 離した後の残りを塗っているストロークは、ビューが隠れると塗り進められないので、その場で塗り終えて確定する（取り残さない）
            crate::view3d::input::settle(&mut self.state);
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
        // メニュー・タブの右クリックで選んだドックの操作（別ウィンドウへ出す・戻す・前に出す）
        self.apply_dock_ops(&ctx);
        self.state.ui.panels = self.detached.index(&self.dock);
        // ウィンドウの縁の上のポインタの形（キャンバスなどが決めた形を上書きする）
        if let Some(direction) = edge.flatten() {
            titlebar::edge_cursor(&ctx, direction);
        }
        crate::selection::dialog::show(&ctx, &mut self.state);
        crate::windows::show(&ctx, &mut self.state);
        crate::prefs::show(&ctx, &mut self.state);
        crate::recovery::window::show(&ctx, &mut self.state);
        // 色のウィンドウ（相手の欄はこのフレームに描いた。変更は次のフレームに相手が受け取る）
        crate::panels::color_window::show_in_app(&ctx, &mut self.state);
        self.state.crash.show(&ctx, self.state.lang);
        // 直前の操作の知らせ（状態の帯の左には出さず、短く出して消える）
        crate::toast::show(&ctx, &mut self.state);
        if self.dialogs {
            self.state.crash.execute_request(self.state.lang);
        }
        // ポップアップが 3D ビューに重なっているか（同じウィンドウに開いたときだけ）
        let view3d_window = self
            .detached
            .window_of(Tab::View3d)
            .map_or(egui::ViewportId::ROOT, |w| {
                self.detached.windows[w].viewport_id()
            });
        let popup_rect = self
            .state
            .popup
            .as_ref()
            .filter(|p| p.state.viewport == view3d_window)
            .map(|p| p.state.rect);
        self.view3d.end_frame(popup_rect);
        // メニューで選んだ Live Link・ファイルの頼みはこのフレームのうちに当てる
        self.handle_requests();
        self.link_exported();
        self.state.link = self.link.view(&self.state);
        // 「保存して更新」: 保存先を選ぶウィンドウも済んだこのフレームの終わりに、保存の結果を見て入れる
        self.state.update_finish_save();
        // 終了・ウィンドウを閉じる: 保存していない変更があれば聞く（ウィンドウを開かない試験では聞かない）
        let close_requested = ctx.input(|i| i.viewport().close_requested());
        self.close_flow(&ctx, close_requested);
    }

    /// 保存の結果を受けて（フレームの初め）、OS の終了を待たせる印を今の保存の有無に合わせる。ウィンドウが見えている間（`ui`）も隠れている間
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

    /// 終了・ウィンドウを閉じる頼みを進める。保存の途中は閉じず（保存を捨てない）、終わるまで待つ。保存が終わったら、その結果の後の状態で、
    /// 保存していない変更があれば聞き、走っている仕事の後始末をして閉じる。
    fn close_flow(&mut self, ctx: &egui::Context, close_requested: bool) {
        if (!self.state.quit && !close_requested) || self.closing {
            return;
        }
        // 離した後の残りを塗っているストロークは、塗り終えて確定してから、保存していない変更を聞く・閉じる
        crate::view3d::input::settle(&mut self.state);
        if self.state.is_saving() {
            // ウィンドウを閉じる頼みは止めて、終わるまで待つ（`quit` に覚える）。画面のスレッドは回し続ける（「応答なし」にならない）
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
        // Live Link の入口: 押したらウィンドウを開く（開いていれば閉じる）
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
        // 別ウィンドウで開いたポップアップは、そのウィンドウのパスが描く
        if open.state.viewport != egui::ViewportId::ROOT {
            self.state.popup = Some(open);
            return;
        }
        // パイは自分で描いて入力を受ける（選んだ項目は閉じてから実行し、別のパイなら開き直す）
        if open.kind == PopupKind::Pie {
            if crate::pie::show(ctx, &mut self.state, &mut open.state) {
                self.state.popup.get_or_insert(open);
            }
            return;
        }
        if open.kind == PopupKind::Transform {
            if crate::objects::transform::show(ctx, &mut self.state, &mut open.state) {
                self.state.popup.get_or_insert(open);
            }
            return;
        }
        // キーを待っている間の受け皿は、設定のウィンドウの「ショートカット」の区分が受けて閉じる
        if open.kind == PopupKind::KeyCapture {
            self.state.popup = Some(open);
            return;
        }
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
    /// Live Link を 1 回まわす: Unity からの頼みを拾って当て、書き出しの返事を書く。ウィンドウが見えている間はフレームの中（`frame_body`）が
    /// 同じことをするので、これを呼ぶのはウィンドウが隠れている間だけ（`eframe::App::logic`）。
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

    /// ウィンドウが隠れている間の 1 回（`eframe::App::logic` が、見えていないときに呼ぶ。試験は、`ui` を回さずにこれを呼んで、隠れたウィンドウの道を
    /// 通す）。
    #[doc(hidden)]
    pub fn tick_hidden(&mut self, ctx: &egui::Context) {
        // 新しい知らせの扱いは `ui` と同じ（隠れている間に出た文は、見えるようになった最初のフレームで知らせとして出る。ここで描き直しは頼まない:
        // 見えないウィンドウを知らせのために回し続けない）
        let prior = self.state.message_begin();
        // 離した後の残りを塗っているストロークは、隠れていると塗り進められないので、その場で塗り終えて確定する
        crate::view3d::input::settle(&mut self.state);
        self.poll_gpu_watch(ctx);
        self.tick_link();
        // 見えないウィンドウでも、Live Link の頼みを拾う間隔で回す（受け付けている間だけ）
        if let Some(wake) = self.link.next_wake() {
            ctx.request_repaint_after(wake);
        }
        // 保存の途中は、隠れていても保存を捨てて閉じない。ウィンドウを閉じる頼み（タスクバーの「閉じる」など）は止めて待ち、終わりを受け、
        // 保存が終わって終了の頼みが残っていれば閉じる流れを進める（見えないウィンドウの保存を、知らせのために回し続けはしない: 保存の間だけ）
        if self.state.is_saving() {
            let close_requested = ctx.input(|i| i.viewport().close_requested());
            self.close_flow(ctx, close_requested);
            self.poll_saving();
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        } else if self.state.quit {
            self.close_flow(ctx, false);
        }
        // 見えないウィンドウでも、受けた要求は実行する（保存の結果を受けた後に）
        self.tick_ops(ctx);
        self.state.message_end(prior);
        self.state.record_message();
    }
}

impl eframe::App for YoluApp {
    /// ウィンドウが隠れている間（Windows の最小化・macOS の覆われたウィンドウ・隠したウィンドウ）は、eframe は egui のパスを回さず `ui` を呼ばない。代わりに、
    /// 描き直しの頼みがあるときだけ（百ミリ秒より速くならない）この `logic` を呼ぶ（eframe 0.36 の `App::logic`）。Live Link の裏のスレッドは
    /// Unity からの知らせのたびに描き直しを頼むので、ここで受け取り・返事をすれば、最小化したまま Unity で Play に入る・スクリプトを
    /// リロードしても、再接続と絵の受け渡しが続く。画面に触れる処理（描く・並べる）は `ui` のまま。見えている間は `ui` がするので何もしない。
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // 別ウィンドウが見えている間は、メインウィンドウが隠れていても eframe が `ui` を回す（別ウィンドウはメインウィンドウのパスの中で描く）
        if ctx.input(|i| i.viewport().visible()) != Some(false) || self.any_detached_visible(ctx) {
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
        // eframe はこのあと、このフレームを描く。このフレームの中（キャンバスの GPU の合成・3D ビューの提出）で失ったと分かったときも、
        // 失ったデバイスで描く前に、ここで拾って終える
        self.watch_point(ui.ctx(), FramePoint::UiEnd);
    }

    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        self.remap_pen_buttons(&mut raw_input.events);
        self.pace_frame(ctx, raw_input);
    }

    /// 正しく終わった: 変更があれば最後の世代を書き、復旧の印を消す（世代は設定の数だけ残す）。
    fn on_exit(&mut self) {
        if self.gpu_lost.is_some() && (self.state.modified || self.state.is_saving()) {
            // GPU を失って終わる: 書き置きの「保存していない作業」の印を消さず（落ちたときと同じ。保存が終わらないまま終えるときも）、
            // 書き込み中の分だけ、期限まで待つ。
            // 遅いディスクで間に合わなくても固まらない（置換は最後の 1 回なので、前の世代が残る）。次の起動の復旧のウィンドウから開ける
            self.state.recovery_wait_within(self.gpu_lost_wait);
        } else {
            self.state.recovery_shutdown();
        }
        // 並びとウィンドウの大きさ・位置を、終わるときに書く（途中で書けていなくても、最後の形を残す）。GPU を失って終わるときは書き直さない:
        // 別ウィンドウを描いている途中（`detached.windows` を取り出している間）に終えることがあり、いまの状態から書くと別ウィンドウが抜ける。
        // 並びは前のフレームまでの `persist_layout` が書いてある（失う物は、最後の数フレームのウィンドウの大きさ・位置の変化だけ）
        if self.gpu_lost.is_none() {
            self.save_layout(false);
        }
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
