//! 保存しない画面の一時の状態（`AppState::ui`）。

use std::collections::HashMap;

use super::OpenPopup;
use crate::engine::LayerId;
use crate::m2::LayerDrag;

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
}
