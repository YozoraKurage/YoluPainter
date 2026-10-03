//! 画面の状態（文書・選んでいるレイヤー・ツール・ブラシ・色・表示）と操作（`Action`）。メニュー・キー・ボタンは同じ `Action` を
//! 通す（試験も同じ道で叩く）。計算は core（`engine`）に任せ、ここは選ぶ・渡す・覚えるだけ。

use std::collections::HashMap;

use egui::{Pos2, Vec2};

use crate::canvas::view::{ViewState, ROTATE_STEP};
use crate::engine::{BlendMode, BrushSettings, Document, LayerId, Rgba8, Stroke};
use crate::ui::menu::PopupState;
use crate::view3d::View3dState;

/// straight の RGBA（0〜1）。
pub type Rgba = [f32; 4];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tool {
    Brush,
    Eraser,
}

impl Tool {
    pub const ALL: [Tool; 2] = [Tool::Brush, Tool::Eraser];
    /// アイコンの名前（tools/<id>）。
    pub fn id(self) -> &'static str {
        match self {
            Tool::Brush => "brush",
            Tool::Eraser => "eraser",
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Tool::Brush => "ブラシ",
            Tool::Eraser => "消しゴム",
        }
    }
    pub fn key(self) -> &'static str {
        match self {
            Tool::Brush => "B",
            Tool::Eraser => "E",
        }
    }
}

/// ブラシの設定（画面の値。Unity 版の BrushState と同じ既定値）。
#[derive(Clone, Debug, PartialEq)]
pub struct BrushState {
    pub radius: f32,
    pub hardness: f32,
    pub spacing: f32,
    pub opacity: f32,
    pub flow: f32,
    pub pressure_size: bool,
    pub pressure_opacity: bool,
    pub pressure_flow: bool,
}

impl Default for BrushState {
    fn default() -> Self {
        BrushState {
            radius: 16.0,
            hardness: 0.8,
            spacing: 0.15,
            opacity: 1.0,
            flow: 1.0,
            pressure_size: true,
            pressure_opacity: true,
            pressure_flow: false,
        }
    }
}

pub const MAX_RADIUS: f32 = 128.0;

impl BrushState {
    /// core に渡す設定。
    pub fn settings(&self, color: Rgba, erase: bool) -> BrushSettings {
        BrushSettings {
            radius: self.radius as f64,
            hardness: self.hardness as f64,
            spacing: self.spacing as f64,
            opacity: self.opacity as f64,
            flow: self.flow as f64,
            color: Rgba8::new(
                to_byte(color[0]),
                to_byte(color[1]),
                to_byte(color[2]),
                to_byte(color[3]),
            ),
            pressure_size: self.pressure_size,
            pressure_opacity: self.pressure_opacity,
            pressure_flow: self.pressure_flow,
            erase,
        }
    }
}

pub fn to_byte(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// Unity の `Color.RGBToHSV`（色相は 0〜1）。
pub fn rgb_to_hsv(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let v = max;
    if max <= 0.0 {
        return (0.0, 0.0, 0.0);
    }
    let s = d / max;
    if d <= 0.0 {
        return (0.0, 0.0, v);
    }
    let mut h = if max == r {
        (g - b) / d
    } else if max == g {
        2.0 + (b - r) / d
    } else {
        4.0 + (r - g) / d
    } / 6.0;
    if h < 0.0 {
        h += 1.0;
    }
    (h, s, v)
}

/// Unity の `Color.HSVToRGB`。
pub fn hsv_to_rgb(h: f32, s: f32, v: f32) -> (f32, f32, f32) {
    if s <= 0.0 {
        return (v, v, v);
    }
    let h6 = (h - h.floor()) * 6.0;
    let i = h6.floor() as i32;
    let f = h6 - i as f32;
    let (p, q, t) = (v * (1.0 - s), v * (1.0 - s * f), v * (1.0 - s * (1.0 - f)));
    match i {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    }
}

/// 16 進（#RRGGBB、大文字。Unity の ToHtmlStringRGB と同じ）。
pub fn to_hex(c: Rgba) -> String {
    format!(
        "{:02X}{:02X}{:02X}",
        to_byte(c[0]),
        to_byte(c[1]),
        to_byte(c[2])
    )
}

/// #RGB・#RRGGBB・#RRGGBBAA（# は無くてもよい。アルファは使わない）。
pub fn parse_hex(s: &str) -> Option<[f32; 3]> {
    let s = s.trim().trim_start_matches('#');
    let digits: Vec<u8> = s
        .chars()
        .map(|c| c.to_digit(16).map(|d| d as u8))
        .collect::<Option<Vec<_>>>()?;
    let rgb = match digits.len() {
        3 => [digits[0] * 17, digits[1] * 17, digits[2] * 17],
        6 | 8 => [
            digits[0] * 16 + digits[1],
            digits[2] * 16 + digits[3],
            digits[4] * 16 + digits[5],
        ],
        _ => return None,
    };
    Some(rgb.map(|v| v as f32 / 255.0))
}

/// 色（メインの色 = 描画色、サブの色 = 背景色）と、色の選び方の覚え（色相は灰色でも失わない）。
#[derive(Clone, Debug, PartialEq)]
pub struct ColorState {
    pub main: Rgba,
    pub sub: Rgba,
    pub hue: f32,
    pub sat: f32,
    pub val: f32,
    picked_for: Option<Rgba>,
    pub recent: Vec<Rgba>,
    /// 色相の円で選ぶ（false なら四角と色相の帯）。
    pub wheel: bool,
}

pub const MAX_RECENT_COLORS: usize = 16;

impl Default for ColorState {
    fn default() -> Self {
        let mut c = ColorState {
            main: [0.0, 0.0, 0.0, 1.0],
            sub: [1.0, 1.0, 1.0, 1.0],
            hue: 0.0,
            sat: 0.0,
            val: 0.0,
            picked_for: None,
            recent: Vec::new(),
            wheel: false,
        };
        c.sync_hsv();
        c
    }
}

impl ColorState {
    /// メインの色から色相・彩度・明度を求め直す（灰色や黒では色相と彩度が決まらないので前の値を残す）。
    pub fn sync_hsv(&mut self) {
        if self.picked_for == Some(self.main) {
            return;
        }
        let (h, s, v) = rgb_to_hsv(self.main[0], self.main[1], self.main[2]);
        if s > 1e-4 && v > 1e-4 {
            self.hue = h;
        }
        if v > 1e-4 {
            self.sat = s;
        }
        self.val = v;
        self.picked_for = Some(self.main);
    }

    pub fn set_main(&mut self, c: Rgba) {
        self.main = c;
        self.picked_for = None;
        self.sync_hsv();
    }

    fn apply_hsv(&mut self) {
        let (r, g, b) = hsv_to_rgb(self.hue, self.sat, self.val);
        self.main = [r, g, b, self.main[3]];
        self.picked_for = Some(self.main);
    }

    /// 彩度×明度の四角で選んだ。
    pub fn pick_sv(&mut self, s: f32, v: f32) {
        self.sync_hsv();
        self.sat = s.clamp(0.0, 1.0);
        self.val = v.clamp(0.0, 1.0);
        self.apply_hsv();
    }

    /// 色相を選んだ（今の彩度と明度は保つ）。
    pub fn set_hue(&mut self, h: f32) {
        self.sync_hsv();
        self.hue = h - h.floor();
        self.apply_hsv();
    }

    pub fn swap(&mut self) {
        std::mem::swap(&mut self.main, &mut self.sub);
        self.picked_for = None;
        self.sync_hsv();
    }

    pub fn defaults(&mut self) {
        self.main = [0.0, 0.0, 0.0, 1.0];
        self.sub = [1.0, 1.0, 1.0, 1.0];
        self.picked_for = None;
        self.sync_hsv();
    }

    /// 描き始めたメインの色を、使った色の先頭に足す（同じ色は前へ移す）。
    pub fn remember(&mut self) {
        let c = self.main;
        self.recent
            .retain(|x| (0..4).any(|i| (x[i] - c[i]).abs() >= 0.002));
        self.recent.insert(0, c);
        self.recent.truncate(MAX_RECENT_COLORS);
    }
}

/// ストロークを描いている入力。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StrokeSource {
    Mouse,
    Pen(u32),
}

/// 回すドラッグ（R ＋ 左ドラッグ）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RotateDrag {
    pub start_angle: f32,
    pub start_pan: Vec2,
    pub swept: f32,
    pub last_pointer_angle: f32,
}

/// キャンバスの入力の途中の状態。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CanvasInput {
    pub stroke: Option<StrokeSource>,
    /// egui の Touch の筆圧（winit が出したとき）。
    pub touch_pressure: Option<f32>,
    pub panning: bool,
    /// Shift ＋ 中ボタンのドラッグで回している。
    pub middle_rotating: bool,
    pub rotating: Option<RotateDrag>,
    pub rotate_key_held: bool,
    pub space_held: bool,
    pub last_pointer: Option<Pos2>,
    /// 最後のストロークの点の数（試験用）。
    pub stroke_points: usize,
}

/// 開いているポップアップの種類。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PopupKind {
    MenuBar(usize),
    BlendMode(LayerId),
    LayerContext(LayerId),
}

#[derive(Clone, Debug, PartialEq)]
pub struct OpenPopup {
    pub kind: PopupKind,
    pub state: PopupState,
}

/// 操作（メニュー・キー・ボタンから）。
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Quit,
    Undo,
    Redo,
    NewLayer,
    DeleteLayer,
    LayerUp,
    LayerDown,
    ToggleVisible(LayerId),
    SetBlend(LayerId, BlendMode),
    StartRename(LayerId),
    ZoomIn,
    ZoomOut,
    FitView,
    RotateLeft,
    RotateRight,
    ResetRotation,
    FlipView,
    ResetLayout,
    SelectTool(Tool),
    SwapColors,
    DefaultColors,
    BrushSmaller,
    BrushLarger,
    ToggleColorWheel,
    /// 3D ビューに試しの立方体を読む。
    LoadDemoModel,
    /// 3D ビューのカメラをモデル全体が見える位置へ。
    FrameModel,
    About,
}

/// 画面の状態の全部。
pub struct AppState {
    pub doc: Document,
    /// 描いているストロークの札（core の `Stroke`。文書を借りないのでフレームをまたいで持つ）。
    pub stroke: Option<Stroke>,
    pub selected_layer: Option<LayerId>,
    pub tool: Tool,
    pub brush: BrushState,
    pub color: ColorState,
    pub view: ViewState,
    /// ステータスバーの知らせ。
    pub message: String,
    /// プロパティの欄のタブ（ブラシ・アルファ・ステンシル・マテリアル）。
    pub property_tab: usize,
    /// 見出しの開閉（キー → 開いているか）。
    pub sections: HashMap<&'static str, bool>,
    pub renaming: Option<LayerId>,
    /// 名前の入力欄がフォーカスを取った後か（外れたら名前の変更を終える）。
    pub rename_started: bool,
    pub layer_scroll: f32,
    pub canvas: CanvasInput,
    pub popup: Option<OpenPopup>,
    /// 前のフレームでポップアップが開いていた（このフレームの押下はキャンバスへ渡さない）。
    pub popup_was_open: bool,
    pub project_name: String,
    /// 保存してから変えたか（M1 は保存が無いので、描いたら立つだけ）。
    pub modified: bool,
    pub reset_layout: bool,
    pub quit: bool,
    /// レイヤーのドラッグの並べ替え（ドラッグ中のレイヤーと、落とす先の隙間 0..=n、上から）。
    pub layer_drag: Option<(LayerId, usize)>,
    /// 最後に描いたキャンバスの表示域（画面の点。試験と外の窓の位置合わせ用）。
    pub canvas_rect: Option<egui::Rect>,
    /// 3D ビュー（モデル・カメラ・描くテクスチャセット・入力）。
    pub view3d: View3dState,
}

/// 新しい文書の既定の大きさ。
pub const DEFAULT_DOCUMENT_SIZE: u32 = 2048;

impl AppState {
    pub fn new(width: u32, height: u32) -> AppState {
        let mut doc = Document::new(width, height).expect("文書の大きさ");
        let first = doc.add_layer("レイヤー 1").ok();
        let _ = doc.clear_history(); // 最初のレイヤーを足したことは取り消せない（空の文書に戻せても意味が無い）
        AppState {
            doc,
            stroke: None,
            selected_layer: first,
            tool: Tool::Brush,
            brush: BrushState::default(),
            color: ColorState::default(),
            view: ViewState::default(),
            message: String::new(),
            property_tab: 0,
            sections: HashMap::new(),
            renaming: None,
            rename_started: false,
            layer_scroll: 0.0,
            canvas: CanvasInput::default(),
            popup: None,
            popup_was_open: false,
            project_name: "名称未設定".into(),
            modified: false,
            reset_layout: false,
            quit: false,
            layer_drag: None,
            canvas_rect: None,
            view3d: View3dState::default(),
        }
    }

    pub fn is_stroking(&self) -> bool {
        self.canvas.stroke.is_some() || self.doc.has_active_stroke()
    }

    pub fn section_open(&self, key: &'static str, default: bool) -> bool {
        *self.sections.get(key).unwrap_or(&default)
    }

    /// 選んでいるレイヤー（消えていれば一番上を選び直す）。
    pub fn ensure_selection(&mut self) {
        let alive = self
            .selected_layer
            .is_some_and(|id| self.doc.layer(id).is_some());
        if !alive {
            self.selected_layer = self.doc.layers().last().map(|l| l.id());
        }
    }

    pub fn layer_name_for_new(&self) -> String {
        format!("レイヤー {}", self.doc.layers().len() + 1)
    }

    /// 今のツールで描くブラシの設定（ペンの消しゴムの端なら消す）。
    pub fn stroke_settings(&self, pen_eraser: bool) -> BrushSettings {
        self.brush
            .settings(self.color.main, self.tool == Tool::Eraser || pen_eraser)
    }

    /// 操作を当てる。描いている最中は、表示と色の操作のほかは断る。
    pub fn apply(&mut self, action: Action) {
        let stroking = self.is_stroking();
        let refuse = |s: &mut AppState| s.message = "描いている間はできません。".into();
        match action {
            Action::Quit => self.quit = true,
            Action::Undo => {
                if stroking {
                    return refuse(self);
                }
                match self.doc.undo() {
                    Ok(true) => {
                        self.message = "取り消しました。".into();
                        self.modified = true;
                    }
                    Ok(false) => {}
                    Err(e) => self.message = e.to_string(),
                }
                self.ensure_selection();
            }
            Action::Redo => {
                if stroking {
                    return refuse(self);
                }
                match self.doc.redo() {
                    Ok(true) => {
                        self.message = "やり直しました。".into();
                        self.modified = true;
                    }
                    Ok(false) => {}
                    Err(e) => self.message = e.to_string(),
                }
                self.ensure_selection();
            }
            Action::NewLayer => {
                if stroking {
                    return refuse(self);
                }
                let name = self.layer_name_for_new();
                if let Ok(id) = self.doc.add_layer_above(&name, self.selected_layer) {
                    self.selected_layer = Some(id);
                    self.modified = true;
                }
            }
            Action::DeleteLayer => {
                if stroking {
                    return refuse(self);
                }
                if let Some(id) = self.selected_layer {
                    if self.doc.layers().len() <= 1 {
                        self.message = "最後のレイヤーは消せません。".into();
                        return;
                    }
                    let index = self.doc.layer_index(id);
                    match self.doc.remove_layer(id) {
                        Ok(()) => {
                            let layers = self.doc.layers();
                            self.selected_layer = index
                                .map(|i| layers[i.saturating_sub(1).min(layers.len() - 1)].id());
                            self.modified = true;
                        }
                        Err(e) => self.message = e.to_string(),
                    }
                }
            }
            Action::LayerUp | Action::LayerDown => {
                if stroking {
                    return refuse(self);
                }
                if let Some(id) = self.selected_layer {
                    if let Some(i) = self.doc.layer_index(id) {
                        let up = action == Action::LayerUp;
                        let to = if up { i + 1 } else { i.wrapping_sub(1) };
                        if to < self.doc.layers().len() {
                            let _ = self.doc.move_layer(id, to);
                            self.modified = true;
                        }
                    }
                }
            }
            Action::ToggleVisible(id) => {
                if stroking {
                    return refuse(self);
                }
                if let Some(visible) = self.doc.layer(id).map(|l| l.visible()) {
                    let _ = self.doc.set_layer_visible(id, !visible);
                    self.modified = true;
                }
            }
            Action::SetBlend(id, mode) => {
                if stroking {
                    return refuse(self);
                }
                if let Err(e) = self.doc.set_layer_blend_mode(id, mode) {
                    self.message = e.to_string();
                    return;
                }
                self.modified = true;
            }
            Action::StartRename(id) => {
                self.renaming = Some(id);
                self.rename_started = false;
            }
            Action::ZoomIn => self
                .view
                .zoom_to(self.view.zoom * 1.25, None, egui::Rect::ZERO),
            Action::ZoomOut => self
                .view
                .zoom_to(self.view.zoom / 1.25, None, egui::Rect::ZERO),
            Action::FitView
            | Action::RotateLeft
            | Action::RotateRight
            | Action::ResetRotation
            | Action::FlipView => {
                if stroking || self.canvas.rotating.is_some() {
                    self.message =
                        "描いている間とドラッグの間は、表示を回したり反転したりしません。".into();
                    return;
                }
                match action {
                    Action::FitView => self.view.fit(),
                    Action::RotateLeft => self.view.rotate_by(-ROTATE_STEP),
                    Action::RotateRight => self.view.rotate_by(ROTATE_STEP),
                    Action::ResetRotation => self.view.set_angle(0.0),
                    _ => self.view.flip_horizontally(),
                }
            }
            Action::ResetLayout => self.reset_layout = true,
            Action::SelectTool(tool) => self.tool = tool,
            Action::SwapColors => self.color.swap(),
            Action::DefaultColors => self.color.defaults(),
            Action::BrushSmaller => self.brush.radius = (self.brush.radius / 1.15).max(0.5),
            Action::BrushLarger => self.brush.radius = (self.brush.radius * 1.15).min(MAX_RADIUS),
            Action::ToggleColorWheel => self.color.wheel = !self.color.wheel,
            Action::LoadDemoModel => {
                if stroking {
                    return refuse(self);
                }
                self.view3d.load_demo();
                self.message = "3D ビューに試しの立方体を読みました。".into();
            }
            Action::FrameModel => {
                if stroking {
                    return refuse(self);
                }
                self.view3d.frame_model();
            }
            Action::About => {
                self.message = format!(
                    "YoluPainter（Rust 版）{} — M1 の試作",
                    env!("CARGO_PKG_VERSION")
                )
            }
        }
    }
}

/// 合成モードの名前（Unity 版の ja.po と同じ）。
pub fn blend_name(mode: BlendMode) -> &'static str {
    match mode {
        BlendMode::Normal => "通常",
        BlendMode::Multiply => "乗算",
        BlendMode::Screen => "スクリーン",
        BlendMode::Overlay => "オーバーレイ",
        BlendMode::Darken => "比較（暗）",
        BlendMode::Lighten => "比較（明）",
        BlendMode::ColorDodge => "覆い焼きカラー",
        BlendMode::ColorBurn => "焼き込みカラー",
        BlendMode::LinearDodge => "覆い焼き（リニア）- 加算",
        BlendMode::LinearBurn => "焼き込み（リニア）",
        BlendMode::HardLight => "ハードライト",
        BlendMode::SoftLight => "ソフトライト",
        BlendMode::VividLight => "ビビッドライト",
        BlendMode::LinearLight => "リニアライト",
        BlendMode::PinLight => "ピンライト",
        BlendMode::HardMix => "ハードミックス",
        BlendMode::Difference => "差の絶対値",
        BlendMode::Exclusion => "除外",
        BlendMode::Subtract => "減算",
        BlendMode::Divide => "除算",
        BlendMode::Hue => "色相",
        BlendMode::Saturation => "彩度",
        BlendMode::Color => "カラー",
        BlendMode::Luminosity => "輝度",
        BlendMode::DarkerColor => "カラー比較（暗）",
        BlendMode::LighterColor => "カラー比較（明）",
        BlendMode::PassThrough => "通過",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hsv_round_trip_and_hex() {
        for c in [
            [1.0, 0.0, 0.0],
            [0.2, 0.6, 0.4],
            [0.5, 0.5, 0.5],
            [0.0, 0.0, 1.0],
        ] {
            let (h, s, v) = rgb_to_hsv(c[0], c[1], c[2]);
            let (r, g, b) = hsv_to_rgb(h, s, v);
            assert!((r - c[0]).abs() < 1e-5 && (g - c[1]).abs() < 1e-5 && (b - c[2]).abs() < 1e-5);
        }
        assert_eq!(to_hex([1.0, 0.5, 0.0, 1.0]), "FF8000");
        assert_eq!(parse_hex("#f80"), Some([1.0, 136.0 / 255.0, 0.0]));
        assert_eq!(parse_hex("00FF00cc"), Some([0.0, 1.0, 0.0]));
        assert_eq!(parse_hex("#12345"), None);
    }

    #[test]
    fn hue_survives_grey_and_black() {
        let mut c = ColorState::default();
        c.set_hue(0.3);
        c.pick_sv(1.0, 1.0);
        c.pick_sv(0.0, 0.0); // 黒にしても
        assert!((c.hue - 0.3).abs() < 1e-6); // 色相は残る
        c.pick_sv(1.0, 1.0);
        let (h, _, _) = rgb_to_hsv(c.main[0], c.main[1], c.main[2]);
        assert!((h - 0.3).abs() < 1e-4);
    }

    #[test]
    fn layer_actions() {
        let mut s = AppState::new(64, 64);
        let first = s.selected_layer.unwrap();
        s.apply(Action::NewLayer);
        let second = s.selected_layer.unwrap();
        assert_ne!(first, second);
        assert_eq!(s.doc.layers().len(), 2);
        s.apply(Action::LayerDown);
        assert_eq!(s.doc.layers()[0].id(), second);
        s.apply(Action::DeleteLayer);
        assert_eq!(s.doc.layers().len(), 1);
        assert_eq!(s.selected_layer, Some(first));
        s.apply(Action::DeleteLayer);
        assert_eq!(s.doc.layers().len(), 1, "最後のレイヤーは消さない");
        s.apply(Action::Undo);
        assert_eq!(s.doc.layers().len(), 2);
    }

    #[test]
    fn recent_colors_move_to_front() {
        let mut c = ColorState::default();
        for v in [0.1, 0.2, 0.1] {
            c.set_main([v, 0.0, 0.0, 1.0]);
            c.remember();
        }
        assert_eq!(c.recent.len(), 2);
        assert_eq!(c.recent[0][0], 0.1);
    }
}
