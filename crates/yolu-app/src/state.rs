//! 画面の状態（文書・選んでいるレイヤー・ツール・ブラシ・色・表示）と操作（`Action`）。メニュー・キー・ボタンは同じ `Action` を
//! 通す（試験も同じ道で叩く）。計算は core（`engine`）に任せ、ここは選ぶ・渡す・覚えるだけ。

use std::collections::HashMap;
use std::path::PathBuf;

use egui::{Pos2, Vec2};

use crate::canvas::view::{ViewState, ROTATE_STEP};
use crate::engine::{AntiAlias, BlendMode, BrushSettings, Document, LayerId, Rgba8, Stroke};
use crate::lang::Lang;
use crate::livelink::{LinkRequest, LinkView};
use crate::m2::{Edit, LayerDrag, M2State, UiOp};
use crate::model::SceneModel;
use crate::notice::Source;
use crate::project::ProjectFile;
use crate::sets::TextureSets;
use crate::shelf::{ShelfOp, ShelfState};
use crate::ui::menu::PopupState;
use crate::view3d::View3dState;

/// straight の RGBA（0〜1）。
pub type Rgba = [f32; 4];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tool {
    Brush,
    Eraser,
    /// バケツ（範囲を 1 回で塗る。範囲は近い色か、モデルの三角形・メッシュの塊・UV アイランド・マテリアル）。
    Fill,
    /// グラデーション（2D のキャンバスをドラッグして、始点から終点へ線形か放射で塗る）。
    Gradient,
    Shape,
    Ruler,
    /// ポリゴン塗りつぶし（押したまま通った範囲を足していく。離して 1 回の Undo）。
    PolygonFill,
    /// スポイト（押した所の値を描画色かマテリアルの値に取る。`eyedrop`）。
    Eyedropper,
    /// 選択のツール（形は `selection`）。
    SelectRect,
    SelectEllipse,
    Lasso,
    Polygon,
    Wand,
    /// ID の色で選択（焼いた ID マップの色から選択範囲を作る）。
    IdSelect,
    /// 選択ペン・選択消し（ブラシで塗るように選択範囲を足す・消す。形は `selection::pen`）。
    SelectPen,
    /// 移動・変形（選んでいるレイヤーをハンドルで移動・拡大縮小・回転。形は `transform`）。
    Move,
    Liquify,
    /// パス（2D のキャンバスとモデルの面の上に引く、編集できる曲線。`pathtool`）。
    Path,
    /// 文字（押した所であとから編集できる文字を打つ。`textlayer`）。
    Text,
}

impl Tool {
    /// 並び順（ツールの帯）。描くツール（ブラシ・消しゴム・バケツ・ポリゴン塗りつぶし）と選ぶツールの間、選ぶツールと移動・変形の間に区切りが入る。
    pub const ALL: [Tool; 19] = [
        Tool::Brush,
        Tool::Eraser,
        Tool::Fill,
        Tool::Gradient,
        Tool::Shape,
        Tool::Ruler,
        Tool::PolygonFill,
        Tool::Eyedropper,
        Tool::SelectRect,
        Tool::SelectEllipse,
        Tool::Lasso,
        Tool::Polygon,
        Tool::Wand,
        Tool::IdSelect,
        Tool::SelectPen,
        Tool::Move,
        Tool::Liquify,
        Tool::Path,
        Tool::Text,
    ];
    /// アイコンの名前（tools/<id>）。
    pub fn id(self) -> &'static str {
        self.def().id
    }
    pub fn name(self) -> &'static str {
        self.name_in(Lang::Ja)
    }
    /// 言語ごとの名前。
    pub fn name_in(self, lang: Lang) -> &'static str {
        self.def().name(lang)
    }
    pub fn key(self) -> &'static str {
        self.def().key
    }
    /// 範囲を塗る・選ぶツール（バケツ・ポリゴン塗りつぶし・ID の色で選択。キャンバスと 3D ビューの入力は `region`）。
    /// ツールの表（`tools`）が持つ。ブラシの否定ではなく並べて書く（ツールが増えたとき、足したツールが黙って範囲のツールになって入力・カーソル・強調の道へ流れない）。
    pub fn is_region(self) -> bool {
        self.def().region
    }
}

/// ブラシの設定（画面の値。アンチエイリアスのほかは Unity 版の BrushState と同じ既定値）。
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
    /// 丸い筆先の縁のアンチエイリアス（core の既定は なし。アプリの既定は 中）。
    pub anti_alias: AntiAlias,
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
            anti_alias: AntiAlias::Medium,
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
            anti_alias: self.anti_alias,
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

pub const MAX_RECENT_COLORS: usize = 64;

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
            wheel: true,
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

/// 回すドラッグ（Alt ＋ 左ドラッグ。R を押しながらの左ドラッグを割り当てたときも）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RotateDrag {
    pub start_angle: f32,
    pub start_pan: Vec2,
    pub swept: f32,
    pub last_pointer_angle: f32,
    /// 押した点（画面の点）。ここから `gesture::CLICK_MOVE` を超えて動くまで回さない（動かさずに離す操作 — クローンの元 — が、小さな揺れで表示を回さないように）。
    pub press: Pos2,
    /// 押した点から遊びを超えて動いたか。
    pub moved: bool,
}

/// キャンバスの入力の途中の状態。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CanvasInput {
    pub stroke: Option<StrokeSource>,
    /// ペンの今の押し（触れてから離すまで。行き先は触れた最初の点で決める）。ペンの `contact` は押している間ずっと続くので、これが
    /// 押し直さない印も兼ねる（押した瞬間に終わるバケツ・ID の色で選択を、押しっぱなしの次の点でまた押さない）。離した点・ウィンドウが
    /// フォーカスを失ったときに下ろす。
    pub pen_press: Option<crate::pen::PenPress>,
    /// Ctrl+Space の拡縮のドラッグ（押した点が中心）。
    pub zooming: Option<crate::gesture::ZoomDrag>,
    /// egui の Touch の筆圧（winit が出したとき）。
    pub touch_pressure: Option<f32>,
    pub panning: bool,
    pub rotating: Option<RotateDrag>,
    /// 表示の回す・パン・拡縮を始めたボタン（そのボタンを離したら終える）。
    pub nav_button: Option<egui::PointerButton>,
    /// 描く・選択などのツールの押しを始めたボタン（そのボタンを離したらツールが終える。無ければ左ボタン）。
    pub tool_button: Option<egui::PointerButton>,
    /// 右ボタン（ペンのサイドボタン）でスポイトを始めている（押したまま動かすと見本が付いてくる。離して決める）。
    pub eyedrop: Option<crate::eyedrop::RightPress>,
    pub rotate_key_held: bool,
    pub space_held: bool,
    pub last_pointer: Option<Pos2>,
    /// 最後のストロークの点の数（試験用）。
    pub stroke_points: usize,
    /// ストロークの最後の入力の時刻（秒。戻さない）。
    pub stroke_time: Option<f64>,
    /// 確定した 2D ストロークの終点（文書座標）。
    pub previous_end: Option<(f64, f64)>,
    /// 描いている 2D ストロークの今の終点（文書座標）。確定すると `previous_end` になる。
    pub current_end: Option<(f64, f64)>,
    /// Shift で始めたストロークの、押した点のぶれの抑えと向きの固定。
    pub shift_hold: Option<ShiftHold>,
    pub ruler_constraint: Option<crate::drafting::Constraint>,
    /// クローンの元を決める組み合わせ（既定は Alt + 左）で押した点とボタン（画面の点。動かさずに離したらクローンの元にする。動かしたら
    /// ドラッグの操作だけ）。
    pub clone_press: Option<(Pos2, egui::PointerButton)>,
    /// 描いているクローンのストロークが使う offset（文書の座標の、描く点から元までのずれ。元の印が今写している点へ動くのに使う）。
    pub clone_offset: Option<yolu_core::glam::DVec2>,
}

/// Shift で押した点（文書座標）のまわりのぶれの抑え。画面の点で一定の距離（`SHIFT_HOLD_POINTS`）を超えて動くまで、点を押した所に留める。
/// 超えたら、`locks` なら最初に動いた向きを 45 度刻みで固定してそれに沿わせ、そうでなければ（前の終点から線を引いた押しなら）
/// 続きは普通に描く。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShiftHold {
    pub origin: (f64, f64),
    pub locks: bool,
    pub direction: Option<(f64, f64)>,
}

/// Shift で押した点から動いたとみなす画面の距離（点）。これより内側のぶれでは、向きを決めず点も動かさない。縮小して見ていても
/// 画面の 1 画素のぶれが数画素の向きに見えないよう、文書の画素でなく画面の点で測る（2D のキャンバスも 3D ビューも同じ）。
pub const SHIFT_HOLD_POINTS: f64 = 8.0;

impl ShiftHold {
    /// Shift で押した点 `origin` のぶれの抑え。前の終点から線を引いた押し（`from_previous`）は、ぶれを超えて動いたら普通に描き、
    /// 前の終点が無い押しは、最初に動いた向きを 45° 刻みで固定する（2D のキャンバスも 3D ビューも同じ）。
    pub fn new(origin: (f64, f64), from_previous: bool) -> ShiftHold {
        ShiftHold {
            origin,
            locks: !from_previous,
            direction: None,
        }
    }

    /// 点 `at`（`origin` と同じ座標）に、ぶれの抑えと向きの固定を当てる。`near` は、`at` が押した点から画面で `SHIFT_HOLD_POINTS` より近いか。
    /// 返すのは当てた点で、None は「続きは普通に描く」（前の終点から線を引いた押しが、ぶれを超えて動いた）。向きは、固定する押しで最初に
    /// ぶれを超えて動いたときの向きを 45° 刻みに丸めて決め、そのあとは変えない。
    pub fn constrain(&mut self, at: (f64, f64), near: bool) -> Option<(f64, f64)> {
        let (ox, oy) = self.origin;
        if self.direction.is_none() && near {
            return Some((ox, oy));
        }
        if !self.locks {
            return None;
        }
        let (dx, dy) = (at.0 - ox, at.1 - oy);
        let (ux, uy) = *self.direction.get_or_insert_with(|| {
            let angle =
                (dy.atan2(dx) / std::f64::consts::FRAC_PI_4).round() * std::f64::consts::FRAC_PI_4;
            (angle.cos(), angle.sin())
        });
        let length = dx * ux + dy * uy;
        Some((ox + length * ux, oy + length * uy))
    }
}

/// 開いているポップアップの種類。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PopupKind {
    MenuBar(usize),
    BlendMode(LayerId),
    LayerContext(LayerId),
    /// レイヤーの一覧の定規のアイコンの右クリック（そのレイヤーの定規の表示・表示の範囲・削除）。
    RulerLayer(LayerId),
    /// M2 のポップアップ（調整レイヤーの種類・ブラシの選択肢・チャンネルの種類など）。
    M2(crate::m2_menu::Popup),
    /// テクスチャセットの右クリック（セットの番号 uid）。
    SetContext(u32),
    /// アセットの棚の素材の右クリック（選んでいる素材）。
    Shelf,
    /// 3D ビューの表示のドロップダウン（マテリアル・中立・チャンネルだけ）。
    View3dShading,
    /// メニューバーの右端の Live Link の入口（状態・Unity・モデルの名前と、待つ／切る）。
    LiveLink,
    /// 重なった UV のベイクのアイランドのメニュー（セット・アイランドの代表の三角形・ベイクのウィンドウの見取り図からか・3D ビューの右クリックからか）。
    BakeIsland {
        set: u32,
        island: usize,
        map: bool,
        surface: bool,
    },
    /// ドックのタブの右クリック（別ウィンドウで開く・ドックに戻す）。
    DockTab(crate::Tab),
    /// オプションバーの左端のモードのドロップダウン（ペイント・編集・ポーズ）。
    Mode,
    /// パイメニュー（中身と途中の状態は `AppState::pie`。開いている間は下の入力を止める）。
    Pie,
    /// 編集・ポーズのモードの G/R/S の途中（中身は `AppState::objects`。開いている間は下の入力を止める）。
    Transform,
    /// ショートカットの設定で、次に押すキー・マウスの組み合わせを待っている間（中身は `AppState::shortcuts`。キーの表・キャンバス・3D ビューの
    /// キーと Esc を止める。描くのは設定のウィンドウの「ショートカット」の区分）。
    KeyCapture,
    /// ショートカットの設定の、ツールのキーの動き方を選ぶ一覧（ツールを選ぶ操作の ID）。
    ToolKeyMode(&'static str),
}

#[derive(Clone, Debug, PartialEq)]
pub struct OpenPopup {
    pub kind: PopupKind,
    pub state: PopupState,
}

/// 操作（メニュー・キー・ボタンから）。
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    ScreenPick(crate::screen_pick::Mode),
    ToggleUvWireframe,
    ShowShortcuts,
    /// 文書を変える M2 の操作（レイヤーの種類・マスク・チャンネルごとの合成・文書のチャンネル。1 つが 1 回の Undo）。
    M2(Edit),
    /// 画面だけの M2 の操作（描くチャンネル・表示・ブラシの選択・言語）。
    M2Ui(UiOp),
    /// マテリアルで塗る（組・値）と、範囲のツール（範囲の種類・許容・塗る/消す・手動の ID の色）の操作。
    Mat(crate::matpaint::MatAction),
    Region(crate::region::RegionAction),
    /// アセットの棚の操作（レイヤーからの保存・文書へ置く・消す・.ylsmart の読み書き。置くのだけが文書を変え、1 回の Undo）。
    Shelf(ShelfOp),
    /// 効果のレイヤー（フィルター・Generator・Anchor）の操作（文書を変える操作は 1 つが 1 回の Undo。行を選ぶ操作は文書を変えない）。
    Fx(crate::fx::FxOp),
    /// ステンシル（画像・読み方・繰り返し・反転・置き場。文書は変えない）。
    Stencil(crate::stencil::StencilOp),
    /// ブラシの一覧（替える・追加・複製・削除・名前・並べ替え・元に戻す。文書は変えない）。
    Brush(crate::brushes::BrushAction),
    /// サブツールの一覧（バケツ・グラデーション・図形などのプリセット。替える・追加・複製・削除・名前・元に戻す・登録。文書は変えない）。
    SubTool(crate::subtool::SubToolAction),
    /// 選択範囲（文書を変える `Edit` は 1 つが 1 回の Undo）と 2 D の対称（画面だけ）の操作。
    Sel(crate::selection::SelAction),
    /// パスのツール（点の操作・ブラシ・組・ラスタライズ。文書を変えるものは 1 つが 1 回の Undo）。
    Path(crate::pathtool::PathAction),
    /// 塗りつぶしレイヤーの画像と投影・デカール・形のグラデーション（文書を変える操作は 1 つが 1 回の Undo。置き場のギズモは画面だけ）。
    Fill(crate::fillfx::FillOp),
    /// レイヤーのメニューの「新規塗りつぶしレイヤー」の画像・デカール・グラデーション（レイヤーの作成と中身の設定は 1 回の Undo）。
    LayerMenu(crate::layermenu::Op),
    /// グラデーションのツール（形・終点・塗る/消す・ドラッグで塗る）。
    Gradient(crate::gradient::GradientOp),
    /// 定規（作る・動かす・消す・移す・表示・スナップ。文書を変えるものは 1 つが 1 回の Undo）。
    Ruler(crate::rulers::RulerAction),
    /// レイヤーの画素のコピー・カット・結合してコピー・ペースト（カットとペーストは 1 回の Undo）。
    Clip(crate::clipboard::ClipAction),
    OpenLogFolder,
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
    /// 3D ビューに試しの立方体を読む（3D ビューの空の状態のボタンと試験の口。メニューには置かない）。
    LoadDemoModel,
    /// 3D ビューのカメラをモデル全体が見える位置へ。
    FrameModel,
    /// 3D ビューの表示の切り替え（何を見せるか・光・環境・トーンマッピング）。
    View3d(crate::view3d::display::Op),
    /// ポーズの変更（FBX を開く・試しの人形・モード・戻す・取り消し）。
    Pose(crate::view3d::pose::PoseAction),
    About,
    /// テクスチャセットを選ぶ（uid）。
    SelectSet(u32),
    /// テクスチャセットの目（uid）。
    ToggleSetVisible(u32),
    /// テクスチャセットの名前を変え始める（uid）。
    StartRenameSet(u32),
    /// Live Link で待ち受ける・やめる。
    ToggleLiveLink,
    /// 新しいプロジェクトにする（保存していない変更があれば聞く）。
    NewProjectDialog,
    NewProject,
    /// .ylp を選んで開く（ファイルのウィンドウ）。
    OpenProjectDialog,
    OpenProject(PathBuf),
    /// 開いた .ylp に上書きで保存する（まだ無ければ別名で）。
    SaveProject,
    SaveProjectAsDialog,
    SaveProjectAs(PathBuf),
    /// メッシュマップのベイク（ウィンドウ・開始・取消・チェック・2D での重ね表示）。
    Bake(crate::bake::BakeAction),
    /// テンプレートの画像の書き出し。
    Export(crate::export::ExportAction),
    /// PSD の読み込みと書き出し。
    Psd(crate::psd::PsdAction),
    /// 配布用に保存（除く物のウィンドウ・保存先・書き込み）。
    Distribute(crate::distribute::DistributeAction),
    /// 新規プロジェクトのウィンドウ・プロジェクトの構成・テクスチャセットの足す・消す。
    Project(crate::newproject::NpAction),
    /// 自動更新（確かめる・更新する・起動時に確かめる設定）。
    Update(crate::update::UpdateAction),
    /// 設定のウィンドウと、設定の値の選び。
    Prefs(crate::prefs::PrefsAction),
    /// 筆圧の調整（設定の「ペン」。全体の筆圧の下限・上限・曲線）。
    Pressure(crate::pen::window::PressureAction),
    /// 復旧（世代の一覧のウィンドウ・開く・捨てる・設定）。
    Recovery(crate::recovery::RecoveryAction),
    /// テクスチャセットの見た目の設定（標準・lilToon と lilToon の値。1 つが 1 回の Undo）。
    Look(crate::look::LookOp),
    /// 重なった UV を 2D のキャンバスに出す・出さない（表示のメニュー）。
    ToggleUvOverlap,
    /// ツールの並び（ツールの列とブラシのグループを作る・消す・名前・並べ替え・最初の並びに戻す。文書は変えない）。
    Tools(crate::toolset::ToolsetAction),
    /// アクション（操作の記録と再生。再生だけが文書を変え、1 回の Undo）。
    Automation(crate::automation::AutomationOp),
    /// ドックのパネルを別ウィンドウへ出す・戻す・前に出す（画面だけ。文書は変えない）。
    Dock(crate::detach::DockOp),
    /// テキストツールとテキストレイヤーの値（打ち始め・打ち終わり・値・フォントのファイル・ラスタライズ。文書を変えるものは 1 つが 1 回の Undo）。
    Text(crate::textlayer::TextAction),
    /// モード（ペイント・編集・ポーズ）と、編集・ポーズのツールの帯のツール（画面だけ。文書は変えない）。
    Mode(crate::mode::ModeAction),
    /// パイメニューを開く（画面だけ）。
    Pie(crate::pie::PieAction),
    /// 3D の視点（軸の視点・選んだセットを収める。画面だけ）。
    View3dNav(crate::view3d::navigation::NavOp),
    /// 編集・ポーズのモードの物（選ぶ・G/R/S・戻す・隠す・消す）。
    Object(crate::objects::ObjectAction),
    /// ツールのキーの動き方を替える（ショートカットの設定。操作の ID と動き方。キーの設定を保存する）。
    ToolKeyMode(&'static str, crate::toolkeys::ToolKeyMode),
}

impl Action {
    /// 診断履歴には種類名だけを渡す。値・名前・パスの整形はしない。
    pub fn kind_name(&self) -> &'static str {
        match self {
            Self::M2(..) => "M2",
            Self::M2Ui(..) => "M2Ui",
            Self::Mat(..) => "Mat",
            Self::Region(..) => "Region",
            Self::Shelf(..) => "Shelf",
            Self::Fx(..) => "Fx",
            Self::Stencil(..) => "Stencil",
            Self::Brush(..) => "Brush",
            Self::SubTool(..) => "SubTool",
            Self::Sel(..) => "Sel",
            Self::Path(..) => "Path",
            Self::Fill(..) => "Fill",
            Self::LayerMenu(..) => "LayerMenu",
            Self::Gradient(..) => "Gradient",
            Self::Clip(..) => "Clip",
            Self::OpenLogFolder => "OpenLogFolder",
            Self::Ruler(..) => "Ruler",
            Self::ScreenPick(..) => "ScreenPick",
            Self::ToggleUvWireframe => "ToggleUvWireframe",
            Self::ShowShortcuts => "ShowShortcuts",
            Self::Quit => "Quit",
            Self::Undo => "Undo",
            Self::Redo => "Redo",
            Self::NewLayer => "NewLayer",
            Self::DeleteLayer => "DeleteLayer",
            Self::LayerUp => "LayerUp",
            Self::LayerDown => "LayerDown",
            Self::ToggleVisible(..) => "ToggleVisible",
            Self::SetBlend(..) => "SetBlend",
            Self::StartRename(..) => "StartRename",
            Self::ZoomIn => "ZoomIn",
            Self::ZoomOut => "ZoomOut",
            Self::FitView => "FitView",
            Self::RotateLeft => "RotateLeft",
            Self::RotateRight => "RotateRight",
            Self::ResetRotation => "ResetRotation",
            Self::FlipView => "FlipView",
            Self::ResetLayout => "ResetLayout",
            Self::SelectTool(..) => "SelectTool",
            Self::SwapColors => "SwapColors",
            Self::DefaultColors => "DefaultColors",
            Self::BrushSmaller => "BrushSmaller",
            Self::BrushLarger => "BrushLarger",
            Self::ToggleColorWheel => "ToggleColorWheel",
            Self::LoadDemoModel => "LoadDemoModel",
            Self::FrameModel => "FrameModel",
            Self::View3d(..) => "View3d",
            Self::Pose(..) => "Pose",
            Self::About => "About",
            Self::SelectSet(..) => "SelectSet",
            Self::ToggleSetVisible(..) => "ToggleSetVisible",
            Self::StartRenameSet(..) => "StartRenameSet",
            Self::ToggleLiveLink => "ToggleLiveLink",
            Self::NewProjectDialog => "NewProjectDialog",
            Self::NewProject => "NewProject",
            Self::OpenProjectDialog => "OpenProjectDialog",
            Self::OpenProject(..) => "OpenProject",
            Self::SaveProject => "SaveProject",
            Self::SaveProjectAsDialog => "SaveProjectAsDialog",
            Self::SaveProjectAs(..) => "SaveProjectAs",
            Self::Bake(..) => "Bake",
            Self::Export(..) => "Export",
            Self::Psd(..) => "Psd",
            Self::Distribute(..) => "Distribute",
            Self::Project(..) => "Project",
            Self::Update(..) => "Update",
            Self::Prefs(..) => "Prefs",
            Self::Pressure(..) => "Pressure",
            Self::Recovery(..) => "Recovery",
            Self::Look(..) => "Look",
            Self::ToggleUvOverlap => "ToggleUvOverlap",
            Self::Tools(..) => "Tools",
            Self::Automation(..) => "Automation",
            Self::Dock(..) => "Dock",
            Self::Text(..) => "Text",
            Self::Mode(..) => "Mode",
            Self::Pie(..) => "Pie",
            Self::View3dNav(..) => "View3dNav",
            Self::Object(..) => "Object",
            Self::ToolKeyMode(..) => "ToolKeyMode",
        }
    }

    /// 今の文書（レイヤー・画素）を変える操作か（読むだけのセットでは断る）。クリップボードの操作は、コピーも読むだけのセットでは
    /// 断る（そのセットの文書は中身の代わりの空の文書で、写しても意味が無い）。
    pub fn edits_document(&self) -> bool {
        if let Action::Fx(op) = self {
            return op.edits_document();
        }
        if let Action::Path(a) = self {
            return a.edits_document();
        }
        matches!(
            self,
            Action::M2(_)
                | Action::LayerMenu(_)
                | Action::Clip(_)
                | Action::Region(crate::region::RegionAction::IdColor(_))
                | Action::Shelf(ShelfOp::Place { .. })
                | Action::Sel(crate::selection::SelAction::Edit(_))
                | Action::Undo
                | Action::Redo
                | Action::NewLayer
                | Action::DeleteLayer
                | Action::LayerUp
                | Action::LayerDown
                | Action::ToggleVisible(_)
                | Action::SetBlend(..)
                | Action::StartRename(_)
        ) || matches!(self, Action::Ruler(op) if op.edits_document())
            || matches!(self, Action::Fill(op) if op.edits_document())
            || matches!(self, Action::Sel(crate::selection::SelAction::Saved(op)) if op.edits_document())
            || matches!(self, Action::Look(op) if op.edits_document())
            || matches!(self, Action::Gradient(op) if op.edits_document())
            || matches!(self, Action::Automation(op) if op.edits_document())
            || matches!(self, Action::Text(op) if op.edits_document())
    }
}

/// 画面の一時の状態（前のフレームの結果・入力の途中。保存しない）。文書を替えると、前の文書を指す物（名前の変更・レイヤーのドラッグ）は
/// `AppState::install_document` が戻す。
#[derive(Debug, Default)]
pub struct UiTemp {
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
    /// 最後に描いたキャンバスの表示域（画面の点。試験と別ウィンドウの位置合わせ用）。
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
    /// プロパティの欄のタブ（ステンシル・レイヤー）の番号。
    pub property_tab: usize,
    /// タブのありか（メインウィンドウ・別ウィンドウ。メニューの「ウィンドウ」とタブの右クリックが読む。`YoluApp` が毎フレーム入れる）。
    pub panels: crate::detach::PanelIndex,
    /// ドックの操作の頼み（メニュー・タブの右クリックから。`YoluApp` が同じフレームのうちに当てる）。
    pub dock_ops: Vec<crate::detach::DockOp>,
}

/// 画面の状態の全部。
pub struct AppState {
    /// 画面の言語（文言は `lang.pick("日本語", "English")`）。
    pub lang: Lang,
    /// M2 の画面の状態（レイヤーの種類・チャンネル・全部入りのブラシ）。
    pub m2: M2State,
    /// マテリアルで塗る設定（ブラシ・バケツ・ポリゴン塗りつぶしが使う）。
    pub mat: crate::matpaint::MaterialPaint,
    /// 範囲のツール（バケツ・ポリゴン塗りつぶし・ID の色で選択）の設定と途中の状態。
    pub region: crate::region::RegionState,
    /// サブツール（バケツ・グラデーション・図形などの設定の組のプリセット。ブラシと消しゴムは `brushes`）。アプリの状態で、.ylp には入れない。
    pub subtools: crate::subtool::SubToolState,
    /// グラデーションセット（グラデーションマップ・塗りつぶしのグラデーションのランプの見本の一覧。利用者の組は設定のフォルダに保存。.ylp には入れない）。
    pub ramp_sets: crate::rampsets::RampSets,
    pub doc: Document,
    /// 文書を別のものに替えた回数（開く・新しく作る・PSD を読み込む・テクスチャセットを切り替える）。キャンバスの表示は、文書 ID が
    /// 同じでも（.ylp や PSD を読み直すと保存した ID が戻る）これが変わったら、前の文書の合成を捨てて作り直す。文書を丸ごと
    /// 置き換える口は [`AppState::document_replaced`] を呼ぶこと。
    pub(crate) doc_epoch: u64,
    /// 描いているストロークの札（core の `Stroke`。文書を借りないのでフレームをまたいで持つ）。
    pub stroke: Option<Stroke>,
    pub selected_layer: Option<LayerId>,
    pub tool: Tool,
    pub brush: BrushState,
    pub color: ColorState,
    pub colorsets: crate::colorsets::ColorSets,
    pub view: ViewState,
    /// 直前の操作の結果と理由（短い文）。状態の帯には出さず、小さな知らせ（`toast`）として短く出して消える。試験が読む。
    /// 書くのは `notice` の `notify`（`info`・`refuse`・`warn`・`fail`）だけ。
    pub message: String,
    /// 最後の知らせ（種類・出どころ。トーストが種類を読む）。
    pub last_notice: Option<crate::notice::Notice>,
    /// 起動してからの注意と失敗（ログのウィンドウが読む）。
    pub notice_log: crate::notice::NoticeLog,
    /// ログのウィンドウの画面の状態（絞り・選んだ行）。
    pub log_view: crate::panels::log::LogView,
    /// 小さな知らせの出し方の状態（どの文をいつから出したか・消したか）。
    pub toast: crate::toast::Toast,
    /// 状態の帯の右端の版・ビルドと使っているメモリ。
    pub usage: crate::usage::Usage,
    /// 画面の一時の状態（名前の変更・レイヤーの欄のスクロールとドラッグ・キャンバスの表示域・見出しの開閉など。保存しない）。
    pub ui: UiTemp,
    pub canvas: CanvasInput,
    pub popup: Option<OpenPopup>,
    pub project_name: String,
    /// 次に名前を付けて保存のウィンドウを開く場所（退避を開いたときの、元の .ylp のフォルダー。保存したら外す。無ければ OS の既定）。
    pub save_folder: Option<std::path::PathBuf>,
    /// ファイルを選ぶウィンドウを、入り口の種類ごとに前に使った場所から開くための覚え（`dialog::places`。試験の状態は設定のフォルダに書かない）。
    pub places: crate::dialog::places::Places,
    /// 開いた・保存した後に変えたか（メニューバーの右の「•」。新規・開くの前に捨ててよいかを聞く）。
    pub modified: bool,
    /// 直前の保存で書き直したテクスチャセット（正本）の数。画面には出さない（試験が、変えていないセットを書き直さないことを確かめる）。
    #[doc(hidden)]
    pub rewritten_sets: usize,
    pub reset_layout: bool,
    pub quit: bool,
    /// テクスチャセット（今のセットの文書は `doc`）。
    pub sets: TextureSets,
    /// 読み込んだモデル（Live Link で受けたもの。3D ビューが読む）。
    pub model: Option<SceneModel>,
    /// Live Link の様子（毎フレーム `LiveLink` から写す。状態の帯とメニューが読む）。
    pub link: LinkView,
    /// Live Link を始める・やめる頼み（`YoluApp` が次に当てる）。
    pub link_request: Option<LinkRequest>,
    /// 今の文書の Live Link の相手（当てた頼み・まとめた Rig との対応。保存・送り直し・書き出しの返事が読む）。
    pub link_target: Option<crate::livelink::LinkTarget>,
    /// 開いた .ylp に残っていた Live Link の相手（`livelink.json`）。`LiveLink` が次のフレームで開き直す。
    pub link_reopen: Option<yolu_protocol::files::Request>,
    /// プロジェクトを替えた回数（新規・開く・作る。`np_project_replaced` が進める）。Live Link の裏の仕事が、始めた時のプロジェクトへだけ入るための目印。
    pub project_epoch: u64,
    /// 外からの操作（MCP のクライアント・コマンドライン）を受けている様子（毎フレーム `McpServer` から写す。状態の帯の印が読む）。
    pub ops: crate::mcp_server::OpsView,
    /// Live Link で入れた「元の絵」のレイヤーの印（レイヤーの欄が読む。保存しない）。
    pub link_originals: crate::livelink::OriginalMarks,
    /// 新規プロジェクトのウィンドウで、利用者が解像度を選んで作ったプロジェクトか（Live Link の元の絵が、最初のセットを元の絵の大きさで作り直してよいかを
    /// 決める。選んだ大きさは元の絵で上書きしない）。起動時の既定・ファイルの「新規」・開いたプロジェクトでは false。
    pub resolution_chosen: bool,
    /// 開いた .ylp（保存先と、保存で残す元の中身）。
    pub project: Option<ProjectFile>,
    /// ファイルのウィンドウを開く頼み（`YoluApp` が開く。試験では開かない）。
    pub dialog_request: Option<DialogRequest>,
    /// 3D ビュー（モデル・カメラ・描くテクスチャセット・入力）。
    pub view3d: View3dState,
    /// クローンの元と設定（2D のキャンバスと 3D ビューで共有する。元は 2D が文書の点、3D が面の点を別々に持つ）。
    pub clone: crate::clone_source::CloneState,
    /// アセットの棚（.ylp の resources）。
    pub shelf: ShelfState,
    /// 個人のライブラリ（フォルダ。アセットの欄が棚と切り替えて見せる）。
    pub library: crate::library::LibraryState,
    /// 選択範囲と 2D の対称の画面の状態（選択範囲そのものは文書が持つ）。
    pub sel: crate::selection::SelState,
    /// メッシュマップのベイク（設定・ウィンドウ・走っている仕事）。
    pub bake: crate::bake::BakeState,
    /// テンプレートの書き出し（パディング・確かめ・結果・走っている仕事）。
    pub export: crate::export::ExportState,
    /// PSD の読み書き（結果・確かめ・走っている仕事）。
    pub psd: crate::psd::PsdState,
    /// 配布用に保存（準備した写し・ウィンドウの選び・走っている仕事）。
    pub distribute: crate::distribute::DistributeState,
    /// .ylp の保存（裏のスレッドの仕事と進み具合。画面のスレッドは頼みと結果の受けだけ）。
    pub save: crate::project::SaveState,
    /// ステンシル（画面に重ねた画像を通して塗る。アプリの状態で、.ylp には入れない）。
    pub stencil: crate::stencil::StencilState,
    /// 効果のレイヤー（選んでいる効果の行・効果の入力の覚え）。
    pub fx: crate::fx::FxState,
    /// 新規プロジェクトのウィンドウ・プロジェクトの構成のウィンドウと、プロジェクトのモデルのファイル。
    pub np: crate::newproject::NpState,
    /// レイヤーの複数選択と、見た目が変わる結合の確かめ。
    pub layer_ops: crate::layerops::LayerOpsState,
    /// 移動・変形のツール（ドラッグの途中・数値・補間）。
    pub transform: crate::transform::TransformState,
    /// スポイトの設定（レイヤーだけか全体か）。
    pub eyedrop: crate::eyedrop::EyedropState,
    /// パスのツール（選んだ点・点のドラッグ・スライダーの途中の値。パスそのものは文書が持つ）。
    pub path: crate::pathtool::PathState,
    /// 塗りつぶしレイヤーの画像と投影・形のグラデーションの画面の状態（置き場のギズモ・ランプの選び）。
    pub fillfx: crate::fillfx::FillFxState,
    /// グラデーションのツールの設定と途中の状態。
    pub gradient: crate::gradient::GradientState,
    pub drafting: crate::drafting::Drafting,
    /// 定規の画面の状態（これから作る定規の設定・スナップの入り切り・選んでいる定規・ドラッグの途中）。定規そのものはレイヤーが持つ。
    pub rulers: crate::rulers::RulerState,
    /// 自動更新（公開鍵を組み込んだビルドだけで動く。聞かずに通信しない）。
    pub update: crate::update::UpdateState,
    /// 設定（メモリの予算・CPU のスレッド・棚の場所など）と設定のウィンドウ。
    pub prefs: crate::prefs::PrefsState,
    pub uv_wireframe: crate::uv_wireframe::Wireframe,
    /// 重なった UV の図（表示と塗りの知らせ）。
    pub uv_overlap: crate::uv_wireframe::overlap::OverlapState,
    pub shortcuts: crate::shortcuts::ShortcutWindow,
    /// 筆圧の調整（設定の「ペン」。枠で描いた線と開いたときの調整。調整そのものは `prefs.settings.pressure`）。
    pub pressure: crate::pen::window::PressureWindow,
    /// 今描いているウィンドウのペンの点のうち、OS に押しを奪われて補った離し（本物の離しではない）のポインタの番号。ウィンドウごとのパスの始めに、そのウィンドウの
    /// ペンの受け口（`PenInput::drain_with_lost`）から入れる。2D と 3D の入力は `pen_release_lost` で見る。アプリの状態で、.ylp には入れない。
    pub pen_lost: Vec<u32>,
    /// クリップボード（アプリの中の写しと、OS のクリップボードとの口。アプリの状態で、.ylp には入れない）。
    pub clip: crate::clipboard::ClipState,
    /// ブラシの一覧（組み込みと利用者のブラシ・ツールごとの覚え・見本・詳細のウィンドウ）。アプリの状態で、.ylp には入れない。
    pub brushes: crate::brushes::BrushesState,
    /// 復旧用の世代の書き置きと復旧のウィンドウ（`recovery`。動かすまでは何もしない）。
    pub crash: crate::crash::window::Report,
    pub recovery: crate::recovery::RecoveryState,
    /// ツールの並び（ツールの列とブラシのグループ。設定のフォルダの tools.json。.ylp には入れない）。
    pub toolset: crate::toolset::ToolsetState,
    /// アクション（操作の記録と再生。置き場は設定のフォルダの actions/。.ylp には入れない）。
    pub automation: crate::automation::Automation,
    /// テキストツール（打っている文字・次の文字の既定・フォントの覚え）。
    pub text: crate::textlayer::TextState,
    /// モード（ペイント・編集・ポーズ）。替えるのは `set_mode`。
    pub mode: crate::mode::EditorMode,
    /// 編集・ポーズのモードのツールの帯で選んでいるツール。
    pub edit_tool: crate::mode::EditTool,
    /// パイメニュー（パイの並びと、開いているもの）。
    pub pie: crate::pie::PieState,
    /// 編集・ポーズのモードの物（選んだ物・隠した物・G/R/S の途中・スナップ）。
    pub objects: crate::objects::ObjectsState,
    /// 利用者のキー・マウス・パイの設定（設定のウィンドウの「ショートカット」の区分が変える。keymap.json）。
    pub keys: crate::keyconfig::KeyConfig,
    /// ツールのキーを押している間だけの切り替え（離したら戻す前のツール。保存しない）。
    pub temp_tool: crate::toolkeys::TempTool,
    /// ツールを替えた回数（`switch_to` が替えるたびに増える。押している間の切り替えが、別の手段で選ばれたかを見る）。
    pub(crate) tool_epoch: u64,
}

/// ファイルのウィンドウの頼み。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DialogRequest {
    New,
    Open,
    SaveAs,
    /// 3D ビューに開くモデル（FBX）を選ぶ。
    OpenModel,
    /// 棚へ読み込む .ylsmart を選ぶ。
    ShelfImport,
    /// 棚の素材（`shelf.export_id`）を書き出す先を選ぶ。
    ShelfExport,
    /// 棚の素材（`shelf.pending_remove`）を消してよいか確かめる。
    ShelfRemove,
    /// ライブラリへ足すファイル（PNG・.ylsmart。複数）を選ぶ。
    LibraryAdd,
    /// ライブラリのファイル（`library.pending_remove`）を消してよいか確かめる。
    LibraryRemove,
    /// ライブラリのフォルダを OS のファイルのウィンドウで開く。
    LibraryReveal,
    /// 書き出しのウィンドウの書き出す先（形が PNG ならファイル、ほかはフォルダ）を選ぶ。
    ExportDestination,
    /// 棚の場所のフォルダを選ぶ。
    PrefsLibraryFolder,
    /// 読み込む PSD を選ぶ。
    PsdImport(crate::psd::PsdTarget),
    /// PSD の書き出し先を選ぶ。
    PsdExport,
    /// 配布用に保存の保存先を選ぶ。
    DistributeSave,
    /// ステンシルの画像（PNG）を選ぶ。
    OpenStencil,
    /// 取り込むブラシのファイル（ABR・GBR・GIH・VBR・PNG・PAT・SUT。複数）を選ぶ。
    ImportBrushes,
    /// 「CLIP STUDIO から」のウィンドウで、サブツールのフォルダ（CLIP STUDIO の外のフォルダも）を手で選ぶ。
    ClipStudioFolder,
    /// 新規プロジェクト・プロジェクトの構成のウィンドウで、モデル（FBX）を選ぶ。
    ProjectModel,
    /// 塗りつぶしの画像にする PNG を選ぶ（棚へ取り込む）。
    FillImage,
    /// 新しい塗りつぶしレイヤーの画像にする PNG を選ぶ（棚へ取り込み、その画像と投影でレイヤーを作る）。
    NewFillImage(yolu_core::fill_image::ProjectionMode),
    /// ディスクキャッシュの置き場所のフォルダを選ぶ。
    PrefsCacheFolder,
    /// ツールの並びを最初の並びに戻してよいか確かめる。
    ToolsetReset,
    /// 利用者のブラシのファイル（`toolset.catalog.pending_delete`）を消してよいか確かめる。
    BrushFileDelete,
    /// 文字のフォントのファイルを選ぶ。
    TextFont,
    /// キーの設定（keymap.json の形）を書き出す先を選ぶ。
    KeymapExport,
    /// 読み込むキーの設定を選ぶ。
    KeymapImport,
}

/// 新しい空の文書（「レイヤー 1」を 1 つ。足したことは取り消せない）。返すのは文書とそのレイヤー。
pub fn blank_document(width: u32, height: u32) -> (Document, Option<LayerId>) {
    blank_document_in(width, height, Lang::Ja)
}

/// `blank_document`の、最初のレイヤーの名前を言語に合わせたもの（新しいレイヤーの名前と同じ言い方）。
pub fn blank_document_in(width: u32, height: u32, lang: Lang) -> (Document, Option<LayerId>) {
    let mut doc = Document::new(width, height).expect("文書の大きさ");
    let first = doc
        .add_layer(&format!("{} 1", lang.pick("レイヤー", "Layer")))
        .ok();
    let _ = doc.clear_history(); // 最初のレイヤーを足したことは取り消せない（空の文書に戻せても意味が無い）
    crate::look::apply_new_set_look(&mut doc);
    (doc, first)
}

/// 新しい文書の既定の大きさ。
pub const DEFAULT_DOCUMENT_SIZE: u32 = 2048;

impl AppState {
    /// 表示の言語を替える。既定の名前のまま（利用者が付けていない。復旧から開いて、まだ保存していない名前を含む）のプロジェクト名・最初のテクスチャセット・
    /// まだ編集していない文書の最初のレイヤーは、新しい言語の名前にする（付けた名前・開いたファイルの名前・編集した文書は変えない）。
    pub fn set_language(&mut self, lang: Lang) {
        let old = self.lang;
        if old == lang {
            return;
        }
        self.lang = lang;
        let untitled = |l: Lang| l.pick("名称未設定", "Untitled");
        if self.project.is_none() && self.project_name == untitled(old) {
            self.project_name = untitled(lang).into();
        }
        // 復旧から開いた文書（保存先がまだ無い）の既定の名前も、言語に追従する
        if self.project.as_ref().is_some_and(|p| !p.is_file())
            && self.project_name == crate::recovery::recovered_name(old)
        {
            self.project_name = crate::recovery::recovered_name(lang).into();
        }
        self.sets.retitle_defaults(old, lang);
        let first_layer = |l: Lang| format!("{} 1", l.pick("レイヤー", "Layer"));
        if self.project.is_none() && !self.doc.can_undo() && self.doc.layers().len() == 1 {
            let layer = &self.doc.layers()[0];
            if layer.name() == first_layer(old) {
                let id = layer.id();
                // 最初のレイヤーを足したことと同じく、名前の付け直しも取り消せない履歴にしない
                if self.doc.set_layer_name(id, &first_layer(lang)).is_ok() {
                    let _ = self.doc.clear_history();
                }
            }
        }
    }

    pub fn new(width: u32, height: u32) -> AppState {
        Self::new_in(width, height, Lang::default())
    }

    /// 言語を決めて作る（最初のレイヤー・テクスチャセット・プロジェクトの名前がその言語になる）。
    pub fn new_in(width: u32, height: u32, lang: Lang) -> AppState {
        let (doc, first) = blank_document_in(width, height, lang);
        let sets = TextureSets::first_in(&doc, lang);
        AppState {
            lang,
            m2: M2State::default(),
            mat: Default::default(),
            region: Default::default(),
            subtools: Default::default(),
            ramp_sets: Default::default(),
            doc,
            doc_epoch: 0,
            stroke: None,
            selected_layer: first,
            tool: Tool::Brush,
            brush: BrushState::default(),
            color: ColorState::default(),
            colorsets: crate::colorsets::ColorSets::default(),
            view: ViewState::default(),
            message: String::new(),
            last_notice: None,
            notice_log: Default::default(),
            log_view: Default::default(),
            toast: crate::toast::Toast::default(),
            usage: crate::usage::Usage::default(),
            ui: UiTemp::default(),
            canvas: CanvasInput::default(),
            popup: None,
            project_name: lang.pick("名称未設定", "Untitled").into(),
            save_folder: None,
            places: Default::default(),
            modified: false,
            rewritten_sets: 0,
            reset_layout: false,
            quit: false,
            sets,
            model: None,
            link: LinkView::default(),
            link_request: None,
            link_target: None,
            link_reopen: None,
            project_epoch: 0,
            ops: Default::default(),
            link_originals: Default::default(),
            resolution_chosen: false,
            project: None,
            dialog_request: None,
            view3d: View3dState::default(),
            clone: Default::default(),
            shelf: ShelfState::default(),
            library: Default::default(),
            sel: crate::selection::SelState::default(),
            bake: Default::default(),
            export: Default::default(),
            psd: Default::default(),
            distribute: Default::default(),
            save: Default::default(),
            stencil: crate::stencil::StencilState::default(),
            fx: crate::fx::FxState::default(),
            np: Default::default(),
            layer_ops: Default::default(),
            transform: Default::default(),
            eyedrop: Default::default(),
            path: Default::default(),
            fillfx: Default::default(),
            gradient: Default::default(),
            drafting: Default::default(),
            rulers: Default::default(),
            update: crate::update::UpdateState::detect(),
            prefs: crate::prefs::PrefsState::default(),
            uv_wireframe: crate::uv_wireframe::Wireframe::default(),
            uv_overlap: Default::default(),
            shortcuts: crate::shortcuts::ShortcutWindow::default(),
            pressure: crate::pen::window::PressureWindow::default(),
            pen_lost: Vec::new(),
            clip: crate::clipboard::ClipState::default(),
            brushes: crate::brushes::BrushesState::default(),
            crash: Default::default(),
            recovery: Default::default(),
            toolset: Default::default(),
            automation: Default::default(),
            text: Default::default(),
            mode: Default::default(),
            edit_tool: Default::default(),
            pie: Default::default(),
            objects: Default::default(),
            keys: Default::default(),
            temp_tool: Default::default(),
            tool_epoch: 0,
        }
    }

    /// 保存の間なら、`what`（断る操作の言い方）と理由（`refusals::saving`）を `message` に書いて true。選ぶウィンドウを開く前の操作が使う。
    fn refuse_while_saving(&mut self, what: &str) -> bool {
        if !self.is_saving() {
            return false;
        }
        let text = self
            .lang
            .with_reason(what, crate::lang::refusals::saving(self.lang));
        self.refuse(Source::Save, text);
        true
    }

    /// 描いている最中か（ストロークと移動・変形のドラッグ。ほかの編集・取り消し・保存を断る）。
    pub fn is_stroking(&self) -> bool {
        self.canvas.stroke.is_some()
            || self.view3d.input.cover.is_some()
            || self.doc.has_active_stroke()
            || self.transform.drag.is_some()
            || self.region.job.is_some()
            || self.region.leftover_drag.is_some()
            || self.path.drag.is_some()
            || self.drafting.drag.is_some()
            || self.rulers.drag.is_some()
            || self
                .view3d
                .input
                .draft
                .is_some_and(|d| d.kind != crate::view3d::draft::DraftKind::Gradient)
    }

    /// パネルの部品の見た目を描き始める前のまま保つ間か（描いている間と、3D ビューでポーズのギズモをドラッグしている間）。
    pub fn holds_panel_look(&self) -> bool {
        self.is_stroking() || self.view3d.pose.drag.is_some()
    }

    /// ドックのタブの見出しをつかんでいる最中か、離した直後のフレームか（このあいだ、ビューは描き始め・回し始めない）。
    pub fn dock_grabbed(&self) -> bool {
        self.ui.dock_grab[0] || self.ui.dock_grab[1]
    }

    /// ツールを替える（どの経路も通る 1 つの口）。ブラシと消しゴムはツールごとに最後のブラシへ（ストロークの最中に替わるなら断って false）。
    /// 替わるときは、途中の選択の形・移動と変形のドラッグ・パスの途中のドラッグと選んだ点を捨てる。`keep_effect` なら、選んでいる効果の行と
    /// 「ID の色」を選んでいる状態は残す（効果の欄から ID マップを読むツールへ移るとき。そうでなければツールの欄へ戻す）。
    pub(crate) fn switch_tool(&mut self, tool: Tool, keep_effect: bool) -> bool {
        let slot = self.toolset.set.slot_for_tool(tool);
        self.switch_to(tool, slot, keep_effect)
    }

    /// ツールを、ツールの列の `slot`（列に無いツールをキーで使うときは None）へ替える。ブラシ・消しゴムのツールは、そのツールの最後のブラシへ
    /// （ストロークの最中にブラシが替わるなら断って false）。編集・ポーズのモードからは、ペイントのモードへ戻る（戻せなければツールも替えない）。
    pub(crate) fn switch_to(
        &mut self,
        tool: Tool,
        slot: Option<crate::toolset::SlotId>,
        keep_effect: bool,
    ) -> bool {
        if !self.set_mode(crate::mode::EditorMode::Paint) {
            return false;
        }
        let changes = tool != self.tool || slot != self.toolset.set.active();
        if changes && !self.brush_for_slot(tool, slot) {
            return false;
        }
        self.toolset.set.set_active(slot);
        if tool != self.tool {
            // 今のツールの設定を覚え、入るツールの今のサブツールの設定を今の設定にする（ブラシと消しゴムは `brush_for_tool` が済ませた）
            self.subtool_leave(self.tool);
            self.sel_tool_changed();
            if !keep_effect {
                self.fx.selected = None; // 選んだ効果の欄はツールを替えたら閉じる
                self.fx.id_pick = None;
            }
            self.transform_cancel_drag();
            self.path_tool_changed();
            self.gradient_cancel_drag();
            self.drafting_cancel();
            self.text_commit();
            self.text.press = None;
            self.subtool_enter(tool);
        }
        self.tool = tool;
        self.tool_epoch += 1;
        true
    }

    /// 描ける先が 3D の面だけか（3D のタブが出ていてモデルがあり、キャンバスのタブは出ていない）。ドックを分けて両方が出ているあいだは、
    /// 2D にも描けるので偽。2D だけの設定（対称など）は、真のあいだだけ無効にする。
    pub fn paints_only_in_3d(&self) -> bool {
        self.view3d.paintable_on_screen() && !self.ui.canvas_visible
    }

    /// 取り消せるか（多角形の点を打っている間は最後の点、ポーズのモードではポーズの取り消し）。
    pub fn can_undo(&self) -> bool {
        if self.sel_has_polygon_point() {
            return true; // 取り消しは最後の点に当たる（`Action::Undo` と同じ順）
        }
        match &self.view3d.pose.session {
            Some(s) if crate::view3d::pose::owns_undo(self) => s.can_undo(),
            _ => self.doc.can_undo(),
        }
    }

    pub fn can_redo(&self) -> bool {
        match &self.view3d.pose.session {
            Some(s) if crate::view3d::pose::owns_undo(self) => s.can_redo(),
            _ => self.doc.can_redo(),
        }
    }

    pub fn section_open(&self, key: &'static str, default: bool) -> bool {
        *self.ui.sections.get(key).unwrap_or(&default)
    }

    /// 選んでいるレイヤー（消えていれば一番上を選び直す）。
    pub fn ensure_selection(&mut self) {
        self.ensure_m2_selection();
        let alive = self
            .selected_layer
            .is_some_and(|id| self.doc.layer(id).is_some());
        if !alive {
            self.selected_layer = self.doc.layers().last().map(|l| l.id());
        }
    }

    pub fn layer_name_for_new(&self) -> String {
        format!(
            "{} {}",
            self.lang.pick("レイヤー", "Layer"),
            self.doc.layers().len() + 1
        )
    }

    /// 今のツールで描くブラシの設定（ペンの消しゴムの端なら消す）。
    pub fn stroke_settings(&self, pen_eraser: bool) -> BrushSettings {
        self.brush
            .settings(self.color.main, self.tool.erases() || pen_eraser)
    }

    /// 操作を当てる（`message` に書かれた文は、前と同じ文でも新しい知らせとして出る）。描いている最中は、表示と色の操作のほかは断る（離した後の残りを塗っている間は、先に確定する）。
    pub fn apply(&mut self, action: Action) {
        // 離した後の残りを塗っている 3D のストローク（確定待ち）は、先に残りを塗って確定してから、この操作を通す（断らない・捨てない）
        crate::view3d::input::settle(self);
        let prior = self.message_begin();
        // 記録中なら、命令にできる操作を記録する（入れ子の操作は外の操作として 1 回）
        let pending = crate::automation::record::before(self, &action);
        self.apply_action(action);
        self.drop_foreign_ruler_selection();
        crate::automation::record::after(self, pending);
        self.message_end(prior);
    }

    fn apply_action(&mut self, action: Action) {
        crate::crash::action(action.kind_name());
        let stroking = self.is_stroking();
        let refuse =
            |s: &mut AppState| s.refuse(Source::Edit, crate::lang::refusals::during_stroke(s.lang));
        // ポーズのモードの取り消し・やり直しは文書を変えない（ポーズの並びを戻す）ので、読むだけのセットでも断らない
        let pose_undo =
            matches!(action, Action::Undo | Action::Redo) && crate::view3d::pose::owns_undo(self);
        if action.edits_document() && !stroking && !pose_undo {
            if let Some(reason) = self.read_only_reason() {
                let text = crate::lang::refusals::read_only_set(self.lang, reason);
                self.refuse(Source::Edit, text);
                return;
            }
        }
        match action {
            Action::OpenLogFolder => {
                self.crash.request = Some(crate::crash::window::Request::Folder)
            }
            Action::ToggleUvWireframe => {
                self.prefs.settings.uv_wireframe = !self.prefs.settings.uv_wireframe
            }
            Action::ToggleUvOverlap => {
                self.prefs.settings.uv_overlap = !self.prefs.settings.uv_overlap
            }
            Action::ShowShortcuts => self.prefs_apply(crate::prefs::PrefsAction::OpenAt(
                crate::prefs::Category::Shortcuts,
            )),
            Action::M2(edit) => self.m2_edit(edit),
            Action::M2Ui(op) => self.m2_ui(op),
            Action::Mat(a) => self.mat_apply(a),
            Action::Look(op) => self.look_apply(op),
            Action::Dock(op) => self.ui.dock_ops.push(op),
            Action::Region(a) => self.region_apply(a),
            Action::Shelf(op) => self.shelf_apply(op),
            Action::Fx(op) => self.fx_apply(op),
            Action::Stencil(op) => self.stencil_op(op),
            Action::Brush(action) => self.brush_action(action),
            Action::Tools(action) => self.toolset_action(action),
            Action::SubTool(action) => self.subtool_action(action),
            Action::Sel(action) => self.sel_action(action),
            Action::Path(a) => self.path_apply(a),
            Action::Text(a) => self.text_apply(a),
            Action::Fill(op) => self.fill_apply(op),
            Action::LayerMenu(op) => self.layer_menu_apply(op),
            Action::Gradient(op) => self.gradient_apply(op),
            // 定規のつまみのドラッグ中などに、文書を変える定規の操作（Ctrl+4 の切り替えなど）が通ると、離したときに古い値で書き戻される。
            // 取り消しと同じく断る
            Action::Ruler(op) if stroking && op.edits_document() => refuse(self),
            Action::Ruler(op) => self.ruler_action(op),
            Action::Clip(action) => self.clip_action(action),
            Action::Quit => self.quit = true,
            Action::Undo => {
                if stroking {
                    return refuse(self);
                }
                // 打っている文字は、打ち終えてから（打った分が 1 回の取り消し）
                self.text_commit();
                // 多角形の途中なら、文書でなく最後の点を取り消す（Photoshop と同じ）
                if self.sel_undo_polygon_point() {
                    return;
                }
                if crate::view3d::pose::owns_undo(self) {
                    return self.apply(Action::Pose(crate::view3d::pose::PoseAction::Undo));
                }
                match self.doc.undo() {
                    Ok(true) => {
                        self.info(Source::Edit, self.lang.pick("取り消しました。", "Undone."));
                        self.modified = true;
                    }
                    Ok(false) => {}
                    Err(e) => self.notify(
                        crate::notice::Kind::of_core(&e),
                        Source::Edit,
                        self.lang.core_error(&e),
                    ),
                }
                self.ensure_selection();
            }
            Action::Redo => {
                if stroking {
                    return refuse(self);
                }
                self.text_commit();
                if crate::view3d::pose::owns_undo(self) {
                    return self.apply(Action::Pose(crate::view3d::pose::PoseAction::Redo));
                }
                match self.doc.redo() {
                    Ok(true) => {
                        self.info(Source::Edit, self.lang.pick("やり直しました。", "Redone."));
                        self.modified = true;
                    }
                    Ok(false) => {}
                    Err(e) => self.notify(
                        crate::notice::Kind::of_core(&e),
                        Source::Edit,
                        self.lang.core_error(&e),
                    ),
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
                if self.has_multiple_layers_selected() {
                    let revision = self.doc.revision();
                    if let Err(e) = self.delete_selected_layers() {
                        self.notify(
                            crate::notice::Kind::of_core(&e),
                            Source::Layer,
                            self.lang.core_error(&e),
                        );
                    }
                    if self.doc.revision() != revision {
                        self.modified = true;
                    }
                    self.ensure_selection();
                    return;
                }
                if let Some(id) = self.selected_layer {
                    // グループは中身ごと消える。何も残らなくなる削除は断る
                    let size = crate::m2::subtree_len(&self.doc, id);
                    if self.doc.layers().len() <= size {
                        self.refuse(
                            Source::Layer,
                            self.lang.pick(
                                "最後のレイヤーは消せません。",
                                "Cannot delete the last layer.",
                            ),
                        );
                        return;
                    }
                    let start = self
                        .doc
                        .layer_index(id)
                        .map(|i| (i + 1).saturating_sub(size));
                    match self.doc.remove_layer(id) {
                        Ok(()) => {
                            let layers = self.doc.layers();
                            self.selected_layer = start
                                .map(|i| layers[i.saturating_sub(1).min(layers.len() - 1)].id());
                            self.set_edit_mask(false);
                            self.modified = true;
                        }
                        Err(e) => self.notify(
                            crate::notice::Kind::of_core(&e),
                            Source::Layer,
                            self.lang.core_error(&e),
                        ),
                    }
                }
            }
            Action::LayerUp | Action::LayerDown => {
                if stroking {
                    return refuse(self);
                }
                if self.has_multiple_layers_selected() {
                    // 選んだレイヤーをまとめて 1 段（それぞれの兄弟の中で。選んだレイヤーどうしは追い越さない）
                    let ids = self.selected_layers();
                    if let Ok(true) = self.doc.step_layers(&ids, action == Action::LayerUp) {
                        self.modified = true;
                    }
                    return;
                }
                if let Some(id) = self.selected_layer {
                    // 同じグループの兄弟の中で動かす（グループの外へは出さない）
                    let parent = self.doc.layer(id).and_then(|l| l.parent());
                    let siblings = self.doc.children_of(parent).unwrap_or_default();
                    if let Some(i) = siblings.iter().position(|v| *v == id) {
                        let up = action == Action::LayerUp;
                        let to = if up { i + 1 } else { i.wrapping_sub(1) };
                        if to < siblings.len() {
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
                    self.notify(
                        crate::notice::Kind::of_core(&e),
                        Source::Layer,
                        self.lang.core_error(&e),
                    );
                    return;
                }
                self.modified = true;
            }
            Action::StartRename(id) => {
                self.ui.renaming = Some(id);
                self.ui.rename_started = false;
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
                    self.refuse(
                        Source::Canvas,
                        crate::lang::refusals::during_stroke_or_drag(self.lang),
                    );
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
            Action::ScreenPick(mode) => crate::screen_pick::request(self, mode),
            Action::ResetLayout => self.reset_layout = true,
            Action::SelectTool(tool) => {
                // 描くツールを選んだら、ペイントのモードへ（`switch_to`）
                self.switch_tool(tool, false);
            }
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
                // 試しの立方体はファイルのモデルではない（プロジェクトのモデルの参照は外す）
                self.np.model_file = None;
                self.info(
                    Source::View3d,
                    self.lang.pick(
                        "3D ビューに試しの立方体を読みました。",
                        "Test cube loaded in the 3D View.",
                    ),
                );
            }
            Action::FrameModel => {
                if stroking {
                    return refuse(self);
                }
                self.view3d.frame_model();
            }
            Action::Pose(a) => crate::view3d::pose::apply_action(self, a),
            Action::View3d(op) => self.view3d.display.apply(op),
            Action::About => {
                self.info(
                    Source::Edit,
                    crate::usage::about_text(self.lang, env!("CARGO_PKG_VERSION")),
                );
            }
            Action::SelectSet(uid) => {
                if let Some(i) = self.sets.index_of(uid) {
                    if let Err(e) = self.switch_set(i) {
                        self.refuse(Source::TextureSet, e);
                    }
                }
            }
            Action::ToggleSetVisible(uid) => self.toggle_set_visible(uid),
            Action::StartRenameSet(uid) => match self.sets.by_uid(uid) {
                Some(set) if set.read_only.is_some() => {
                    let reason = set.read_only.as_deref().unwrap_or_default();
                    let text = crate::lang::refusals::read_only_set(self.lang, reason);
                    self.refuse(Source::TextureSet, text);
                }
                Some(_) => {
                    self.ui.renaming_set = Some(uid);
                    self.ui.rename_set_started = false;
                }
                None => {}
            },
            Action::ToggleLiveLink => {
                self.link_request = Some(if self.link.is_on() {
                    LinkRequest::Stop
                } else {
                    LinkRequest::Start
                })
            }
            Action::NewProjectDialog => {
                if stroking {
                    return refuse(self);
                }
                // 保存の間は、選んでから断るのではなく、ウィンドウを開く前に断る
                if self.refuse_while_saving(self.lang.pick(
                    "新しいプロジェクトを作れません",
                    "Cannot create a new project",
                )) {
                    return;
                }
                self.dialog_request = Some(DialogRequest::New)
            }
            Action::NewProject => {
                if stroking {
                    return refuse(self);
                }
                crate::project::new_into(self);
            }
            Action::OpenProjectDialog => {
                if stroking {
                    return refuse(self);
                }
                if self.refuse_while_saving(
                    self.lang
                        .pick("プロジェクトを開けません", "Cannot open a project"),
                ) {
                    return;
                }
                self.dialog_request = Some(DialogRequest::Open)
            }
            Action::SaveProjectAsDialog => {
                if stroking {
                    return refuse(self);
                }
                if self.refuse_while_saving(
                    self.lang
                        .pick("プロジェクトを保存できません", "Cannot save the project"),
                ) {
                    return;
                }
                self.dialog_request = Some(DialogRequest::SaveAs)
            }
            Action::OpenProject(path) => {
                if stroking {
                    return refuse(self);
                }
                crate::project::open_into(self, &path);
            }
            Action::SaveProject => {
                if stroking {
                    return refuse(self);
                }
                // 保存の間は、保存先を選ぶウィンドウ（まだファイルが無いプロジェクト）も開かずに断る
                if self.refuse_while_saving(
                    self.lang
                        .pick("プロジェクトを保存できません", "Cannot save the project"),
                ) {
                    return;
                }
                match self
                    .project
                    .as_ref()
                    .filter(|p| p.is_file())
                    .map(|p| p.path().to_path_buf())
                {
                    Some(path) => crate::project::save_from(self, &path),
                    None => self.dialog_request = Some(DialogRequest::SaveAs),
                }
            }
            Action::SaveProjectAs(path) => {
                if stroking {
                    return refuse(self);
                }
                crate::project::save_from(self, &path);
            }
            Action::Bake(a) => self.bake_apply(a),
            Action::Export(a) => self.export_apply(a),
            Action::Psd(a) => self.psd_apply(a),
            Action::Distribute(a) => self.distribute_apply(a),
            Action::Project(a) => self.np_apply(a),
            Action::Update(a) => self.update_apply(a),
            Action::Prefs(a) => self.prefs_apply(a),
            Action::Pressure(a) => self.pressure_apply(a),
            Action::Recovery(a) => self.recovery_apply(a),
            Action::Automation(op) => self.automation_apply(op),
            Action::Mode(op) => self.mode_apply(op),
            Action::Pie(op) => self.pie_apply(op),
            Action::View3dNav(op) => crate::view3d::navigation::apply(self, op),
            Action::Object(op) => self.objects_apply(op),
            Action::ToolKeyMode(command, mode) => {
                self.keys.set_tool_mode(command, mode);
                self.keys_changed();
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
    fn a_shift_hold_stays_on_the_pressed_point_then_locks_the_first_direction_to_45_degrees() {
        let mut hold = ShiftHold {
            origin: (100.0, 100.0),
            locks: true,
            direction: None,
        };
        // 押した点のぶれ（画面の点で数画素まで）は、向きも点も動かさない
        assert_eq!(hold.constrain((103.0, 101.0), true), Some((100.0, 100.0)));
        assert_eq!(hold.direction, None);
        // 超えて動いた最初の向きを 45° 刻みに丸めて固定し、その線の上へ落とす
        let (x, y) = hold.constrain((140.0, 112.0), false).unwrap();
        assert!(
            (x - 140.0).abs() < 1e-9 && y.abs() > 0.0 && (y - 100.0).abs() < 1e-9,
            "{x} {y}"
        );
        let direction = hold.direction.expect("向きを決めた");
        assert!((direction.0 - 1.0).abs() < 1e-9 && direction.1.abs() < 1e-9);
        // 向きを決めたあとは、近くへ戻っても向きを変えない
        let (x, y) = hold.constrain((130.0, 150.0), true).unwrap();
        assert!(
            (x - 130.0).abs() < 1e-9 && (y - 100.0).abs() < 1e-9,
            "{x} {y}"
        );
        // 斜めは 45°
        let mut diagonal = ShiftHold {
            origin: (0.0, 0.0),
            locks: true,
            direction: None,
        };
        let (x, y) = diagonal.constrain((50.0, 40.0), false).unwrap();
        assert!(
            (x - 45.0).abs() < 1e-9 && (y - 45.0).abs() < 1e-9,
            "{x} {y}"
        );
    }

    #[test]
    fn a_shift_hold_from_a_previous_end_lets_go_when_it_moves_past_the_jitter() {
        let mut hold = ShiftHold {
            origin: (100.0, 100.0),
            locks: false,
            direction: None,
        };
        assert_eq!(hold.constrain((102.0, 100.0), true), Some((100.0, 100.0)));
        // 前の終点からの線は押した点で終わっている: 超えて動いたら、続きは普通に描く
        assert_eq!(hold.constrain((140.0, 112.0), false), None);
    }

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
