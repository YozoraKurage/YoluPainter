//! 保存しない画面の一時の状態（`AppState::ui`）。

use std::collections::HashMap;

use super::OpenPopup;
use crate::engine::LayerId;
use crate::m2::{LayerDrag, MaskDrag};

/// 画面の一時の状態（前のフレームの結果・開いているポップアップ・入力の途中。保存しない）。状態の側から画面への頼みは `Outbox`。文書を替えると、前の文書を指す物（名前の変更・レイヤーのドラッグ）は
/// `AppState::install_document` が戻す。
#[derive(Debug, Default)]
pub struct UiState {
    /// 名前を変えているレイヤー。
    pub renaming: Option<LayerId>,
    /// 名前の入力欄がフォーカスを取った後か（外れたら名前の変更を終える）。
    pub rename_started: bool,
    /// レイヤーの欄のスクロール。
    pub layer_scroll: f32,
    /// レイヤーのドラッグの並べ替え（ドラッグ中のレイヤーと、落とす先の隙間 0..=n、上から）。
    pub layer_drag: Option<LayerDrag>,
    /// マスクのサムネイルのドラッグ（別のレイヤーへ動かす・Alt で写す。並べ替えの `layer_drag` とは別）。
    pub mask_drag: Option<MaskDrag>,
    /// 名前を変えているテクスチャセット（uid）と、入力欄がフォーカスを取った後か。
    pub renaming_set: Option<u32>,
    pub rename_set_started: bool,
    /// テクスチャセットの欄のスクロール。
    pub set_scroll: f32,
    /// 最後に描いたキャンバスの表示域（画面の点。テストと別ウィンドウの位置合わせ用）。
    pub canvas_rect: Option<egui::Rect>,
    /// キャンバスのタブが画面に出ているか（前のフレームの結果。`canvas_drawn` はこのフレームで描いたか）。ドックを分けると、
    /// キャンバスと 3D ビューが同時に出る。プロパティの欄が、描く先が 3D だけのときに限って 2D の設定を無効にする。
    pub canvas_visible: bool,
    pub canvas_drawn: bool,
    /// UI を隠している（Tab。`view.toggle_ui`）。メニューの帯とビュー（キャンバスと 3D ビュー）の本体だけを残し、ツールの列・オプションバー・
    /// 状態の帯・ビューでないパネル・ビューのタブの見出しと、ビューの中の部品（キャンバスの右上の部品・3D ビューの左上のドロップダウンと右上のアイコン・
    /// 軸のギズモ）を出さない。ドックの並びは変えず、保存もしない。ビューの上に重ねる新しい部品は、この印を見て出さない。
    pub hide_ui: bool,
    /// 最後にキャンバスのタブを描いたフレームの番号（メインウィンドウの `Context::cumulative_frame_nr_for`。キャンバスを別ウィンドウへ出しても、メインウィンドウの番号）。
    /// タブが後ろにあるあいだは進まない。
    pub canvas_frame: Option<u64>,
    /// ドックのタブの見出しをつかんで動かしている（前のフレームと、その前のフレーム。離した直後のフレームも入る）。つかんでいる間と
    /// 離した直後は、キャンバスと 3D ビューが描き始め・回し始めない。`YoluApp::frame` がタブの見出しの押しから毎フレーム入れる。
    pub dock_grab: [bool; 2],
    /// 前のフレームでポップアップが開いていた（このフレームの押下はキャンバスへ渡さない）。
    pub popup_was_open: bool,
    /// 見出しの開閉（キー → 開いているか）。
    pub sections: HashMap<&'static str, bool>,
    /// プロパティの欄のタブ（`panels::properties::TAB_STENCIL`・`TAB_LAYER`）。初めはレイヤー。画面の並びの順ではなく名前の値（保存しない）。
    pub property_tab: usize,
    /// タブのありか（メインウィンドウ・別ウィンドウ。メニューの「ウィンドウ」とタブの右クリックが読む。`YoluApp` が毎フレーム入れる）。
    pub panels: crate::detach::PanelIndex,
    /// ドックの操作の頼み（メニュー・タブの右クリックから。`YoluApp` が同じフレームのうちに当てる）。
    pub dock_ops: Vec<crate::detach::DockOp>,
    /// 開いているポップアップ。
    pub popup: Option<OpenPopup>,
    /// ログのウィンドウの画面の状態（絞り・選んだ行）。
    pub log_view: crate::panels::log::LogView,
    /// ショートカットの設定のウィンドウ。
    pub shortcuts: crate::shortcuts::ShortcutWindow,
    /// 筆圧の調整（設定の「ペン」。枠で描いた線と開いたときの調整。調整そのものは `prefs.settings.pressure`）。
    pub pressure: crate::pen::window::PressureWindow,
    /// 保存していない変更の問い（アプリの中のウィンドウ。聞いている間だけ Some。`discard`）。開いている間は、キーの割り当て・キャンバスの入力を止める
    /// （`windows::modal_open`）ので、後ろの文書は変わらない。
    pub discard_ask: Option<crate::discard::Ask>,
    /// 復旧のウィンドウ（世代の一覧・選び・捨てる前の確かめ・位置。開いている間だけ Some。書き置きそのものは `services.recovery`）。
    pub recovery_window: Option<crate::recovery::window::WindowState>,
    /// 更新の初回の問いのウィンドウと、更新の準備のウィンドウの位置（`update::window`）。
    pub update_ask_offset: egui::Vec2,
    pub update_ready_offset: egui::Vec2,
    /// 書き出しの置き換えの確かめと、書き終えた結果のウィンドウの位置（`windows`）。
    pub export_confirm_offset: egui::Vec2,
    pub export_report_offset: egui::Vec2,
    /// PSD の読み書きの結果・置き換えの確かめ・書き出しの設定・書く前の確かめのウィンドウの位置（`windows`・`psd_export`）。
    pub psd_report_offset: egui::Vec2,
    pub psd_confirm_offset: egui::Vec2,
    pub psd_options_offset: egui::Vec2,
    pub psd_notes_offset: egui::Vec2,
    /// 配布用に保存のウィンドウと、置き換えの確かめのウィンドウの位置（`distribute::window`）。
    pub distribute_window_offset: egui::Vec2,
    pub distribute_replace_offset: egui::Vec2,
    /// テクスチャセットを消す確かめのウィンドウの位置（`newproject::window`）。
    pub np_remove_offset: egui::Vec2,
    /// レイヤーの統合の確かめのウィンドウの位置（`windows`）。
    pub layer_ops_confirm_offset: egui::Vec2,
    /// 設定のウィンドウの見出しをドラッグして動かした量（画面の真ん中からの。`prefs::window`）。
    pub prefs_offset: egui::Vec2,
    /// 選択範囲の量のウィンドウを見出しで動かした量（開くたびに戻す。`selection::dialog`）。
    pub sel_dialog_offset: egui::Vec2,
    /// 選択範囲の帯をドラッグでずらした量（初めの位置から。選択範囲を外すと戻る。`selection::bar`）。
    pub sel_bar_offset: egui::Vec2,
    /// 最後に 3D ビューを描いたウィンドウ（G/R/S は、このウィンドウのキーとポインタで動かす。`panels::view3d` が入れる）。
    pub view3d_viewport: Option<egui::ViewportId>,
    /// 新規プロジェクト・構成のウィンドウで開いているドロップダウン（`newproject::window`）。ウィンドウを開くとき
    /// （`NpWindow` を作るとき）とモデルを選び直したときに閉じる。
    pub np_dropdown: Option<(crate::newproject::Dropdown, crate::ui::menu::PopupState)>,
    /// 書き出しのウィンドウの位置（`export::window`）。
    pub export_window_offset: egui::Vec2,
    /// PSD の取り込みの確かめのウィンドウの位置（`psd_import`）。
    pub psd_import_window_offset: egui::Vec2,
    /// ブラシの詳細のウィンドウを見出しのドラッグで動かした量（画面の真ん中からの。初めて開いたときに置き場を決める。
    /// `panels::brush_detail`）。
    pub brushes_detail_offset: egui::Vec2,
    /// 影響元設定のウィンドウの位置（見出しのドラッグでずれた量。`panels::brush_rows`）。
    pub brushes_dynamics_offset: egui::Vec2,
    /// 「CLIP STUDIO から」のウィンドウの位置（見出しのドラッグでずれた量。`panels::brush_clipstudio`）。
    pub brushes_csp_offset: egui::Vec2,
    /// ブラシを追加するウィンドウを見出しのドラッグで動かした量（画面の真ん中からの。初めて開いたときに置き場を決める。
    /// `panels::brush_catalog`）。
    pub toolset_catalog_offset: egui::Vec2,
    /// ライブラリの格子の範囲（このフレームで描いたときだけ入る。ウィンドウに落としたファイルが格子の上かを見る。プロジェクトの
    /// アセット・チャンネルのタブを開いている間は入らない。`panels::assets`）。
    pub library_grid_rect: Option<egui::Rect>,
    /// 前のフレームにブラシの一覧を描いた範囲（描かなかったフレームでは消える。ファイルを落とした所が一覧の上かを見る。
    /// `panels::brushes`）。
    pub brushes_list_rect: Option<egui::Rect>,
    /// 前のフレームにステンシルの画像の箱を描いた範囲（押せるときだけ。落とした PNG が箱の上かを見る。`panels::stencil_props`）。
    pub stencil_image_rect: Option<egui::Rect>,
    /// 前のフレームのツールの列のボタンの矩形（名前の欄・アイコンの格子を、その右に出す。`toolset::ui`）。ツールの並びを
    /// 初めに戻すと空にする。
    pub toolset_slot_rects: Vec<(crate::toolset::SlotId, egui::Rect)>,
    /// ブラシの見本の画像とテクスチャ（`brushes::sample`。ツールのプロパティ・詳細のウィンドウ・一覧の行・アセットの欄が出す）。
    pub samples: crate::brushes::sample::SampleCache,
    /// キャンバスの右上の部品の、開いている欄とスクロール（`canvas::bar`）。
    pub canvas_bar: crate::canvas::bar::BarState,
}
