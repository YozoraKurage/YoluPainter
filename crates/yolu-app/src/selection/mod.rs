//! 選択範囲のツール（画面の側）。形（矩形・楕円・投げ縄・多角形・自動選択）を core の `SelectionMask` にして、今の選択範囲と
//! 作成方法（新規・追加・削除・共通）で組み合わせ、1 回の Undo で文書に置く。メニュー（すべて・解除・反転・拡張・縮小・境界・ぼかし・くっきり）も、
//! core の `SelectionMask` の操作を呼ぶだけ。選択範囲は文書（`Document`）が持つので、セットごとに別で、Undo・.ylp の保存と読み込みも文書に付く。
//!
//! - `canvas`: キャンバスの入力（ドラッグ・クリック・Esc）と、選択の縁（点線が流れる表示）・ドラッグ中の形・対称の軸の表示
//! - `outline`: 選択範囲の縁の線分（点線の元）
//! - `menu`・`props`・`dialog`: 選択メニュー・オプションバーとプロパティの欄・量を聞く小さなウィンドウ
//! - `io`: .ylp の `selection.bin` との受け渡し
//! - `pen`: 選択ペン・選択消し（ブラシで塗るように選択範囲を足す・消す）。`quick`: クイックマスク（選択範囲を赤い重ねで見せ、ブラシ・
//!   消しゴムで直す）。`overlay`: マスクの量を色つきの重ねで見せる。`saved`: 名前を付けて残した選択範囲（文書の持ち物。.ylp に保存）
//!
//! 文書を変える操作は `Action::Sel(SelAction::Edit(..))`（1 つが 1 回の Undo。描いている間と読むだけのセットでは断る）、画面だけの
//! 操作は `SelAction::Ui`（Undo に入らない）。対称は定規（`rulers`）が持つ文書の値で、ストロークを始めるときにブラシへ写して固める
//! （途中で変えても、そのストロークには効かない）。

pub mod bar;
pub mod canvas;
pub mod dialog;
pub mod io;
pub mod menu;
mod ops;
pub mod outline;
pub mod overlay;
pub mod pen;
pub mod props;
pub mod quick;
pub mod saved;
pub mod shape;

use crate::engine::{
    CanvasSymmetry, CoreError, DVec2, LayerKind, SelectionCombine, SelectionMask,
    DEFAULT_WORKING_BUDGET_BYTES, MAX_MODIFY_RADIUS,
};
use crate::lang::Lang;
use crate::notice::Source;
use crate::state::{Action, AppState, StrokeSource, Tool};

/// 選択範囲を変える操作（半径を取るものと、取らないもの）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ModifyKind {
    /// どの画素も、半径の円の中の最大の量になる。
    Grow,
    Shrink,
    /// 縁の帯（拡張 − 縮小）。
    Border,
    Feather,
    /// 半分以上の量を全部に、ほかを 0 に（半径は取らない）。
    Sharpen,
}

impl ModifyKind {
    pub const ALL: [ModifyKind; 5] = [
        ModifyKind::Grow,
        ModifyKind::Shrink,
        ModifyKind::Border,
        ModifyKind::Feather,
        ModifyKind::Sharpen,
    ];

    pub fn name(self, lang: Lang) -> &'static str {
        match self {
            ModifyKind::Grow => lang.pick("拡張", "Grow"),
            ModifyKind::Shrink => lang.pick("縮小", "Shrink"),
            ModifyKind::Border => lang.pick("境界線", "Border"),
            ModifyKind::Feather => lang.pick("境界をぼかす", "Feather"),
            ModifyKind::Sharpen => lang.pick("境界をくっきり", "Sharpen Edge"),
        }
    }

    /// 半径を取るか。
    pub fn uses_radius(self) -> bool {
        self != ModifyKind::Sharpen
    }

    /// キャンバスの縁を固定するかの設定が効くか（拡張は外へ広がるだけなので効かない）。
    pub fn uses_edge_lock(self) -> bool {
        matches!(
            self,
            ModifyKind::Shrink | ModifyKind::Border | ModifyKind::Feather
        )
    }

    /// 半径の意味（ツールチップ）。
    pub fn tooltip(self, lang: Lang) -> &'static str {
        match self {
            ModifyKind::Grow => lang.pick(
                "半径の円の中の最大の量にする",
                "Largest amount within a circle of the radius",
            ),
            ModifyKind::Shrink => lang.pick(
                "半径の円の中の最小の量にする",
                "Smallest amount within a circle of the radius",
            ),
            ModifyKind::Border => lang.pick(
                "縁の帯だけを残す（拡張 − 縮小）",
                "A band around the edge: Grow minus Shrink",
            ),
            ModifyKind::Feather => lang.pick(
                "縁をぼかす（ガウス。標準偏差は半径 ÷ 3.5）",
                "Soften the edge (Gaussian blur, σ = radius / 3.5)",
            ),
            ModifyKind::Sharpen => lang.pick(
                "半分以上選ばれた所を全部選び、ほかを外す",
                "At least half selected becomes fully selected",
            ),
        }
    }
}

/// 選択範囲を変える操作（文書を変える。1 つが 1 回の Undo。選択範囲が変わらないときは段を積まない）。
#[derive(Clone, Debug, PartialEq)]
pub enum SelEdit {
    All,
    Clear,
    Invert,
    Modify {
        kind: ModifyKind,
        radius: u32,
        edge_lock: bool,
    },
    /// 画素の座標（左下が原点）の矩形。中心がこの中にある画素が選ばれる。
    Rect {
        x0: i64,
        y0: i64,
        x1: i64,
        y1: i64,
        mode: SelectionCombine,
    },
    Ellipse {
        cx: f64,
        cy: f64,
        rx: f64,
        ry: f64,
        mode: SelectionCombine,
    },
    /// 角を丸めた長方形（画素の座標。角の半径は短い辺の半分までに丸める。縁の滑らかさは `SelState::antialias`）。
    RoundRect {
        x0: i64,
        y0: i64,
        x1: i64,
        y1: i64,
        radius: u32,
        mode: SelectionCombine,
    },
    /// 投げ縄・多角形（キャンバスの座標の点。3 つ未満なら何も選ばない）。
    Polygon {
        points: Vec<(f64, f64)>,
        mode: SelectionCombine,
    },
    /// できあがった形（選択ペンの 1 ストロークの被覆）を、今の選択範囲と組み合わせる。
    Shape {
        mask: SelectionMask,
        mode: SelectionCombine,
    },
    /// 残しておいた選択範囲（今の文書のもの。番号は `saved_selections` の並び）を、今の選択範囲と組み合わせる。
    Recall {
        index: usize,
        mode: SelectionCombine,
    },
    /// 自動選択（許し幅・隣接・全レイヤーは `SelState` の今の値）。種は画素の座標（左下が原点）。2 つ以上（対称定規の写し）なら、それぞれから
    /// 求めた範囲の和を、今の選択範囲と 1 回で組み合わせる。
    Wand {
        seeds: Vec<(u32, u32)>,
        mode: SelectionCombine,
    },
    /// 選択範囲を描画色で塗りつぶす（選んでいるレイヤー。マスクを描いているならマスク）。
    Fill,
    /// 選択範囲の画素を消す（アルファを減らす。マスクなら隠す）。
    Erase,
    /// 選択範囲の画素を新しいレイヤーとして元の位置にコピーする（クリップボードは変えない）。
    ToNewLayer,
    /// 選択範囲を選んでいるレイヤーのマスクにする（外を隠す）。
    ToMask,
}

/// 画面だけの選択の操作（Undo に入らない）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelUiOp {
    /// 選択のツールの組み合わせ方（オプションバー）。
    Combine(SelectionCombine),
    /// 量を聞くウィンドウを開く（選択範囲が無ければ開かない）。
    OpenAmount(ModifyKind),
    /// ウィンドウの値で適用して閉じる。
    ApplyAmount,
    CancelAmount,
    /// 選択範囲の下のボタンの帯を出す・出さない。
    Bar(bool),
    /// クイックマスクを入れる・切る（None は切り替え）。
    QuickMask(Option<bool>),
    /// 選択ペンのツールの基本（false が選択ペン、true が選択消し。Shift・Ctrl は押している間だけ替える）。
    PenErase(bool),
    /// ツールプロパティの作成方法を、4 つ出す（true）か、共通を畳んで 3 つ出す（false）か（設定に覚える）。
    AllModes(bool),
}

/// `Action::Sel` の中身。
#[derive(Clone, Debug, PartialEq)]
pub enum SelAction {
    Edit(SelEdit),
    Ui(SelUiOp),
    /// 名前を付けて残した選択範囲（残す・名前を変える・消す・ウィンドウ。文書の持ち物で、1 回の Undo。.ylp に保存）。
    Saved(saved::SavedOp),
}

/// 量を聞くウィンドウの状態。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AmountDialog {
    pub kind: ModifyKind,
    pub radius: u32,
    pub edge_lock: bool,
}

/// ドラッグで決める形（矩形・楕円・投げ縄）の途中。
#[derive(Clone, Debug, PartialEq)]
pub struct ShapeDrag {
    pub tool: Tool,
    pub source: StrokeSource,
    /// 押した画面の点（クリックとドラッグを分ける）。
    pub start_screen: egui::Pos2,
    /// キャンバスの座標。
    pub start: (f64, f64),
    pub current: (f64, f64),
    /// 投げ縄の点（1 画素以上離れたものだけ）。
    pub lasso: Vec<(f64, f64)>,
    /// 画面で動いた最大の距離（クリックかドラッグか）。
    pub moved: f32,
}

/// 選択範囲と対称の画面の状態。
pub struct SelState {
    /// 選択のツールの組み合わせ方（キーの修飾が無いとき）。
    pub combine: SelectionCombine,
    /// 自動選択の許し幅（0〜255）・隣接・全レイヤー（選んだレイヤーでなく合成から選ぶ）。
    pub tolerance: u8,
    pub contiguous: bool,
    pub all_layers: bool,
    /// 自動選択: 効いている 2D の対称定規の写しの全部の点から選ぶ（3D ビューでは効かない）。
    pub snap_symmetry: bool,
    /// 拡張・縮小・境界・ぼかしの半径（画素）と、キャンバスの縁を固定するか。
    pub radius: u32,
    pub edge_lock: bool,
    pub dialog: Option<AmountDialog>,
    /// ウィンドウを見出しで動かした量。
    pub dialog_offset: egui::Vec2,
    pub drag: Option<ShapeDrag>,
    /// 3D ビューで画面の上に引いている形と、打っている多角形の点（表示域の画面の点。`view3d::select`）。
    pub view3d: crate::view3d::select::Draft,
    /// 多角形の途中の点（キャンバスの座標）と、ポインタの今の位置（ゴムの線）。
    pub polygon: Vec<(f64, f64)>,
    pub polygon_hover: Option<(f64, f64)>,
    /// 最後に押した時刻と点（ダブルクリックで多角形を閉じる）。
    pub last_press: Option<(f64, egui::Pos2)>,
    /// 形のドラッグを押し始めたときの修飾（離したときと見比べて、追加か縦横比の固定かを分ける）。
    pub press_modifiers: egui::Modifiers,
    /// 選択のツールを押したボタン（組み合わせ方を、組み合わせの表のそのボタンの行で引く）。
    pub press_button: egui::PointerButton,
    /// ペンが触れている間の ID（ペンの触れる・離すを押す・離すにする）。
    pub pen_down: Option<u32>,
    /// 描いているストロークに固めた対称（写しのカーソルはこれ）。
    pub stroke_symmetry: Option<CanvasSymmetry>,
    /// 縁の滑らかさ（楕円・なげなわ・多角形・角丸の長方形）。切ると縁は 0 か 255 だけ。
    pub antialias: bool,
    /// 長方形・楕円を常に縦横比 1:1（正方形・正円）にする（Shift を押しているあいだの形を、押さなくても）。
    pub fixed_ratio: bool,
    /// 長方形・楕円を、押した点が中心になるように広げる（Alt を押しているあいだの形を、押さなくても）。
    pub from_center: bool,
    /// 長方形の角の丸め（画素。0 は丸めない）。
    pub corner_radius: u32,
    /// 選択ペンのツールの基本: false が選択ペン、true が選択消し。
    pub pen_erase: bool,
    /// 動いているペンのストローク（選択ペンのツール・クイックマスクのブラシ）。
    pub pen: Option<pen::ActivePen>,
    /// 最後に来たペンの点（ID・筆圧・消しゴムの端）。選択ペンが筆圧と消しゴムの端を使う。
    pub pen_note: Option<(u32, f32, bool)>,
    /// クイックマスク（選択範囲を赤い重ねで見せ、ブラシ・消しゴムを選択ペン・選択消しとして使う）。
    pub quick: bool,
    /// 重ね表示のタイル（クイックマスクの赤・選択ペンの途中）。
    pub quick_overlay: overlay::TileOverlay,
    pub pen_overlay: overlay::TileOverlay,
    /// 残した選択範囲の合計（プロジェクト全体）の上限（バイト）と、1 回のペンのストロークの作業の上限。試験が小さい値に替えて断りを通す。
    /// 残した選択範囲そのものは文書（core の `Document`）が持つ。
    pub saved_budget: u64,
    pub pen_budget: u64,
    /// 残した選択範囲のウィンドウ（開いていれば）。
    pub saved_window: Option<saved::SavedWindow>,
    /// 縁の点線を流す（試験は止めて、同じ絵を撮る）。
    pub animate: bool,
    /// 帯をドラッグでずらした量（初めの位置から。選択範囲を外すと戻る）。
    pub bar_offset: egui::Vec2,
    /// 最後に描いたとき、縁を一部しか描かなかったか（縁が多すぎて打ち切った・1 画面の上限を超えた）。
    pub edge_partial: bool,
    outline: Option<OutlineCache>,
    bounds: Option<BoundsCache>,
}

/// 選択範囲を囲む画素の矩形（選択範囲が変わったときだけ求め直す）。
struct BoundsCache {
    mask: SelectionMask,
    bounds: Option<(u32, u32, u32, u32)>,
}

struct OutlineCache {
    mask: SelectionMask,
    runs: Vec<outline::Run>,
    truncated: bool,
}

impl Default for SelState {
    fn default() -> Self {
        SelState {
            combine: SelectionCombine::Replace,
            tolerance: 32,
            contiguous: true,
            all_layers: false,
            snap_symmetry: true,
            radius: 5,
            edge_lock: false,
            dialog: None,
            dialog_offset: egui::Vec2::ZERO,
            drag: None,
            view3d: Default::default(),
            polygon: Vec::new(),
            polygon_hover: None,
            last_press: None,
            press_modifiers: egui::Modifiers::NONE,
            press_button: egui::PointerButton::Primary,
            pen_down: None,
            stroke_symmetry: None,
            antialias: true,
            fixed_ratio: false,
            from_center: false,
            corner_radius: 0,
            pen_erase: false,
            pen: None,
            pen_note: None,
            quick: false,
            quick_overlay: overlay::TileOverlay::default(),
            pen_overlay: overlay::TileOverlay::default(),
            saved_budget: saved::SAVED_BUDGET_BYTES,
            pen_budget: pen::PEN_BUDGET_BYTES,
            saved_window: None,
            animate: true,
            bar_offset: egui::Vec2::ZERO,
            edge_partial: false,
            outline: None,
            bounds: None,
        }
    }
}

impl SelState {
    /// 選択ペンの筆圧（0〜1）。ペンならその点の筆圧、マウスならタッチの筆圧（無ければ 1）。
    pub fn pen_pressure(&self, source: StrokeSource, touch: Option<f32>) -> f32 {
        match source {
            StrokeSource::Pen(id) => self
                .pen_note
                .filter(|(n, _, _)| *n == id)
                .map_or(1.0, |(_, p, _)| p),
            StrokeSource::Mouse => touch.unwrap_or(1.0),
        }
    }

    /// 途中の形（ドラッグ・多角形）を捨てる。何かあったか。
    pub fn cancel_drafts(&mut self) -> bool {
        // クイックマスクのブラシのストロークは描くストロークの流れ（`canvas.stroke`）が終わらせる。選択ペンのツールのものは、ほかの形と同じく捨てる
        let pen = self.pen.as_ref().is_some_and(|a| !a.quick);
        if pen {
            self.pen = None;
        }
        let surface = self.view3d.cancel();
        let any = self.drag.is_some() || !self.polygon.is_empty() || pen || surface;
        self.drag = None;
        self.polygon.clear();
        self.polygon_hover = None;
        self.last_press = None;
        self.pen_down = None;
        any
    }

    /// 選択範囲に量のある画素を全部含む矩形（x0, y0, x1, y1。半開区間、キャンバスの画素の座標）。何も選んでいなければ None。
    /// 選択範囲が変わったときだけ求め直す（タイル 1 枚ずつ量を見て、量のある画素の端まで詰める）。
    pub fn bounds_of(&mut self, mask: &SelectionMask) -> Option<(u32, u32, u32, u32)> {
        let fresh = self.bounds.as_ref().is_some_and(|c| c.mask.same_as(mask));
        if !fresh {
            self.bounds = Some(BoundsCache {
                mask: mask.clone(),
                bounds: exact_bounds(mask),
            });
        }
        self.bounds.as_ref().and_then(|c| c.bounds)
    }

    /// 選択範囲の縁の線分（選択範囲が変わったときだけ求め直す）。打ち切ったかも返す。
    pub fn outline_of(&mut self, mask: &SelectionMask) -> (&[outline::Run], bool) {
        let fresh = self.outline.as_ref().is_some_and(|c| c.mask.same_as(mask));
        if !fresh {
            let (runs, truncated) = outline::outline(mask);
            self.outline = Some(OutlineCache {
                mask: mask.clone(),
                runs,
                truncated,
            });
        }
        let cache = self.outline.as_ref().expect("上で入れた");
        (&cache.runs, cache.truncated)
    }
}

fn exact_bounds(mask: &SelectionMask) -> Option<(u32, u32, u32, u32)> {
    let ts = mask.tile_size();
    let mut tile = vec![0u8; (ts * ts) as usize];
    let mut found: Option<(u32, u32, u32, u32)> = None;
    for coord in mask.tile_coords() {
        if mask.copy_tile(coord, &mut tile).is_err() {
            continue;
        }
        let (ox, oy) = (coord.x * ts, coord.y * ts);
        for y in 0..ts {
            if oy + y >= mask.height() {
                break;
            }
            let row = &tile[(y * ts) as usize..((y + 1) * ts) as usize];
            let first = row.iter().position(|&a| a > 0);
            let last = row.iter().rposition(|&a| a > 0);
            if let (Some(first), Some(last)) = (first, last) {
                let (x0, x1) = (ox + first as u32, ox + last as u32 + 1);
                let (y0, y1) = (oy + y, oy + y + 1);
                found = Some(match found {
                    None => (x0, y0, x1, y1),
                    Some((a, b, c, d)) => (a.min(x0), b.min(y0), c.max(x1), d.max(y1)),
                });
            }
        }
    }
    found
}

/// 組み合わせ方の名前。
pub fn combine_name(lang: Lang, mode: SelectionCombine) -> &'static str {
    match mode {
        SelectionCombine::Replace => lang.pick("新規", "New"),
        SelectionCombine::Add => lang.pick("追加", "Add"),
        SelectionCombine::Subtract => lang.pick("削除", "Subtract"),
        SelectionCombine::Intersect => lang.pick("共通", "Intersect"),
    }
}

/// 組み合わせ方のツールチップ（名前と、割り当ての表の修飾。割り当てが無ければ名前だけ）。
pub fn combine_tooltip(lang: Lang, mode: SelectionCombine) -> String {
    use crate::keymap::Operation;
    let operation = match mode {
        SelectionCombine::Replace => None,
        SelectionCombine::Add => Some(Operation::SelectionAdd),
        SelectionCombine::Subtract => Some(Operation::SelectionSubtract),
        SelectionCombine::Intersect => Some(Operation::SelectionIntersect),
    };
    name_with_modifier(lang, combine_name(lang, mode), operation)
}

/// 名前に、選択の修飾の割り当て（`keymap` の "selection" の行）の入力を添える（「追加（Shift）」・英語は「Add (Shift)」）。
/// 行が無ければ名前だけ。
pub fn name_with_modifier(
    lang: Lang,
    name: &str,
    operation: Option<crate::keymap::Operation>,
) -> String {
    match operation.and_then(|op| modifier_text(lang, op)) {
        Some(key) => lang.pick(format!("{name}（{key}）"), format!("{name} ({key})")),
        None => name.to_owned(),
    }
}

/// 選択の修飾の行（`op`）の入力の文字。修飾だけ（「Shift」「Ctrl」「Shift+Ctrl」。Mac は Cmd）。
/// 左ボタン以外・修飾の無い行は、ボタンも含めた文（「Shift+右ボタン」）。外した行は None。
fn modifier_text(lang: Lang, op: crate::keymap::Operation) -> Option<String> {
    use crate::shortcuts::gestures::{key_label, Binding};
    let keymap = crate::keymap::current();
    let g = keymap
        .gestures()
        .iter()
        .find(|g| g.scope == "selection" && g.operation == op && g.starts)?;
    let mac = cfg!(target_os = "macos");
    let mut parts = Vec::new();
    if g.shift {
        parts.push("Shift");
    }
    if g.ctrl {
        parts.push(if mac { "Cmd" } else { "Ctrl" });
    }
    if g.alt {
        parts.push("Alt");
    }
    if g.button == egui::PointerButton::Primary && !g.click && !parts.is_empty() {
        return Some(parts.join("+"));
    }
    Some(key_label(
        &Binding {
            scope: g.scope,
            held: None,
            modifiers: egui::Modifiers {
                alt: g.alt,
                shift: g.shift,
                ctrl: g.ctrl,
                command: g.ctrl,
                ..egui::Modifiers::NONE
            },
            button: g.button,
            operation: g.operation,
            click: g.click,
        },
        lang,
    ))
}

/// 押したボタンとキーの修飾から組み合わせ方（既定は左ボタンの Shift で足す・Ctrl で引く・両方で重ねる。無ければオプションバーの値。
/// 組み合わせは `keymap::GESTURES` の表の selection の行）。
pub fn combine_of(
    base: SelectionCombine,
    button: egui::PointerButton,
    modifiers: egui::Modifiers,
) -> SelectionCombine {
    combine_for(button, &modifiers).unwrap_or(base)
}

/// 押したボタンと修飾が、選択の行（足す・引く・重ねる）に当たるなら、その組み合わせ方。
pub fn combine_for(
    button: egui::PointerButton,
    modifiers: &egui::Modifiers,
) -> Option<SelectionCombine> {
    use crate::keymap::Operation;
    match crate::keymap::gesture("selection", button, modifiers, false) {
        Some(Operation::SelectionAdd) => Some(SelectionCombine::Add),
        Some(Operation::SelectionSubtract) => Some(SelectionCombine::Subtract),
        Some(Operation::SelectionIntersect) => Some(SelectionCombine::Intersect),
        _ => None,
    }
}

fn dvec(points: &[(f64, f64)]) -> Vec<DVec2> {
    points.iter().map(|&(x, y)| DVec2::new(x, y)).collect()
}

impl AppState {
    /// 選択範囲の操作を当てる（`Action::Sel`）。
    pub fn sel_action(&mut self, action: SelAction) {
        match action {
            SelAction::Edit(edit) => self.sel_edit(edit),
            SelAction::Ui(op) => self.sel_ui(op),
            SelAction::Saved(op) => self.sel_saved(op),
        }
    }

    /// 選択範囲を変える（描いている間と読むだけのセットは `Action::apply` が先に断る）。断られたら何も変えず、理由をステータスバーへ。
    pub fn sel_edit(&mut self, edit: SelEdit) {
        if self.is_stroking() {
            self.refuse(
                Source::Selection,
                crate::lang::refusals::during_stroke(self.lang),
            );
            return;
        }
        let revision = self.doc.revision();
        match self.sel_apply(edit) {
            Ok(text) => self.info(Source::Selection, text),
            Err(e) => self.refuse(Source::Selection, e),
        }
        if self.doc.revision() != revision {
            self.modified = true;
        }
    }

    /// 選択範囲を `next` にする。今と同じ中身なら Undo の段を積まず、false を返す。
    fn set_selection_if_changed(&mut self, next: Option<SelectionMask>) -> Result<bool, CoreError> {
        let next = next.filter(|m| !m.is_empty());
        if next.as_ref() == self.doc.selection() {
            return Ok(false);
        }
        self.doc.set_selection(next)?;
        Ok(true)
    }

    /// 新しい形を今の選択範囲と組み合わせて置く。
    fn combine_shape(
        &mut self,
        shape: SelectionMask,
        mode: SelectionCombine,
    ) -> Result<String, String> {
        let lang = self.lang;
        let next = match self.doc.selection() {
            Some(current) if mode != SelectionCombine::Replace => Some(
                current
                    .combine(&shape, mode)
                    .map_err(|e| lang.core_error(&e))?,
            ),
            _ => (mode != SelectionCombine::Subtract).then_some(shape),
        };
        let changed = self
            .set_selection_if_changed(next)
            .map_err(|e| lang.core_error(&e))?;
        Ok(if self.doc.selection().is_none() {
            lang.pick("何も選ばれていません。", "Nothing is selected.")
                .into()
        } else if !changed {
            lang.pick("選択範囲は変わりません。", "The selection did not change.")
                .into()
        } else {
            format!(
                "{}: {}",
                lang.pick("選択範囲", "Selection"),
                combine_name(lang, mode)
            )
        })
    }

    /// 縁の滑らかさの設定を形に当てる（切っていれば、半分以上の量を全部に・ほかを 0 にする）。
    pub(crate) fn edge_of(&self, shape: SelectionMask) -> SelectionMask {
        if self.sel.antialias {
            shape
        } else {
            shape.sharpen()
        }
    }

    /// 自動選択の基準のレイヤー（選んだレイヤー。ラスターでない・全レイヤーなら None で、チャンネルの合成）。
    fn wand_layer(&self) -> Option<crate::engine::LayerId> {
        if self.sel.all_layers {
            return None;
        }
        let id = self.selected_layer?;
        let layer = self.doc.layer(id)?;
        (layer.kind() == LayerKind::Raster).then_some(id)
    }

    fn sel_apply(&mut self, edit: SelEdit) -> Result<String, String> {
        let lang = self.lang;
        let none =
            || -> String { lang.pick("選択範囲がありません。", "No selection.").into() };
        match edit {
            SelEdit::All => {
                let all = SelectionMask::all(&self.doc);
                let changed = self
                    .set_selection_if_changed(Some(all))
                    .map_err(|e| lang.core_error(&e))?;
                Ok(if changed {
                    lang.pick("すべてを選択しました。", "Selected all.").into()
                } else {
                    lang.pick("すでにすべて選択しています。", "Already all selected.")
                        .into()
                })
            }
            SelEdit::Clear => {
                if self.doc.selection().is_none() {
                    return Ok(none());
                }
                self.doc
                    .clear_selection()
                    .map_err(|e| lang.core_error(&e))?;
                Ok(lang.pick("選択を解除しました。", "Deselected.").into())
            }
            SelEdit::Invert => {
                let Some(current) = self.doc.selection().cloned() else {
                    return Ok(none());
                };
                self.set_selection_if_changed(Some(current.invert()))
                    .map_err(|e| lang.core_error(&e))?;
                Ok(if self.doc.selection().is_none() {
                    lang.pick("反転して、何も残りません。", "Nothing is left selected.")
                        .into()
                } else {
                    lang.pick("選択範囲を反転しました。", "Inverted the selection.")
                        .into()
                })
            }
            SelEdit::Modify {
                kind,
                radius,
                edge_lock,
            } => {
                let Some(current) = self.doc.selection().cloned() else {
                    return Ok(none());
                };
                let r = radius.min(MAX_MODIFY_RADIUS);
                let budget = DEFAULT_WORKING_BUDGET_BYTES;
                let next = match kind {
                    ModifyKind::Grow => current.grow(r, budget),
                    ModifyKind::Shrink => current.shrink(r, edge_lock, budget),
                    ModifyKind::Border => current.border(r, edge_lock, budget),
                    ModifyKind::Feather => current.feather(r as f64, edge_lock, budget),
                    ModifyKind::Sharpen => Ok(current.sharpen()),
                }
                .map_err(|e| lang.core_error(&e))?;
                let changed = self
                    .set_selection_if_changed(Some(next))
                    .map_err(|e| lang.core_error(&e))?;
                let name = kind.name(lang);
                Ok(if self.doc.selection().is_none() {
                    format!(
                        "{name}: {}",
                        lang.pick("何も残りません。", "nothing is left selected.")
                    )
                } else if !changed {
                    format!("{name}: {}", lang.pick("変わりません。", "no change."))
                } else if kind.uses_radius() {
                    format!("{name}: {r} px")
                } else {
                    format!("{name}{}", lang.pick("。", "."))
                })
            }
            SelEdit::Rect {
                x0,
                y0,
                x1,
                y1,
                mode,
            } => {
                let shape = SelectionMask::rectangle(&self.doc, x0, y0, x1, y1);
                self.combine_shape(shape, mode)
            }
            SelEdit::Ellipse {
                cx,
                cy,
                rx,
                ry,
                mode,
            } => {
                let shape = SelectionMask::ellipse(&self.doc, cx, cy, rx, ry)
                    .map_err(|e| lang.core_error(&e))?;
                let shape = self.edge_of(shape);
                self.combine_shape(shape, mode)
            }
            SelEdit::RoundRect {
                x0,
                y0,
                x1,
                y1,
                radius,
                mode,
            } => {
                let points = shape::rounded_rect_points(x0, y0, x1, y1, radius);
                let shape =
                    SelectionMask::polygon(&self.doc, &points).map_err(|e| lang.core_error(&e))?;
                let shape = self.edge_of(shape);
                self.combine_shape(shape, mode)
            }
            SelEdit::Polygon { points, mode } => {
                let shape = SelectionMask::polygon(&self.doc, &dvec(&points))
                    .map_err(|e| lang.core_error(&e))?;
                let shape = self.edge_of(shape);
                self.combine_shape(shape, mode)
            }
            SelEdit::Shape { mask, mode } => self.combine_shape(mask, mode),
            SelEdit::Recall { index, mode } => {
                let mask = self.saved_mask(index)?;
                self.combine_shape(mask, mode)
            }
            SelEdit::Fill => self.sel_fill(false),
            SelEdit::Erase => self.sel_fill(true),
            SelEdit::ToNewLayer => self.sel_to_new_layer(),
            SelEdit::ToMask => self.sel_to_mask(),
            SelEdit::Wand { seeds, mode } => {
                let layer = self.wand_layer();
                let mut shape: Option<SelectionMask> = None;
                for (x, y) in seeds {
                    let part = SelectionMask::magic_wand(
                        &self.doc,
                        layer,
                        self.m2.paint_channel,
                        x,
                        y,
                        self.sel.tolerance,
                        self.sel.contiguous,
                        DEFAULT_WORKING_BUDGET_BYTES,
                    )
                    .map_err(|e| lang.core_error(&e))?;
                    shape = Some(match shape {
                        Some(sum) => sum
                            .combine(&part, SelectionCombine::Add)
                            .map_err(|e| lang.core_error(&e))?,
                        None => part,
                    });
                }
                let Some(shape) = shape else {
                    return Err(lang.pick("種がありません。", "No seed.").into());
                };
                self.combine_shape(shape, mode)
            }
        }
    }

    /// 画面だけの選択の操作。
    pub fn sel_ui(&mut self, op: SelUiOp) {
        match op {
            SelUiOp::Combine(mode) => self.sel.combine = mode,
            SelUiOp::OpenAmount(kind) => {
                if self.is_stroking() {
                    self.refuse(
                        Source::Selection,
                        crate::lang::refusals::during_stroke(self.lang),
                    );
                } else if self.doc.selection().is_none() {
                    self.refuse(
                        Source::Selection,
                        self.lang.pick("選択範囲がありません。", "No selection."),
                    );
                } else if let Some(reason) = self.read_only_reason() {
                    self.refuse(
                        Source::Selection,
                        crate::lang::refusals::read_only_set(self.lang, reason),
                    );
                } else {
                    self.sel.dialog = Some(AmountDialog {
                        kind,
                        radius: self.sel.radius,
                        edge_lock: self.sel.edge_lock,
                    });
                    self.sel.dialog_offset = egui::Vec2::ZERO;
                }
            }
            SelUiOp::ApplyAmount => {
                if let Some(d) = self.sel.dialog.take() {
                    self.sel.radius = d.radius;
                    self.sel.edge_lock = d.edge_lock;
                    self.apply(Action::Sel(SelAction::Edit(SelEdit::Modify {
                        kind: d.kind,
                        radius: d.radius,
                        edge_lock: d.edge_lock,
                    })));
                }
            }
            SelUiOp::CancelAmount => self.sel.dialog = None,
            SelUiOp::Bar(on) => self.prefs.settings.selection_bar = on,
            SelUiOp::QuickMask(on) => self.quick_mask(on),
            SelUiOp::PenErase(erase) => self.sel.pen_erase = erase,
            SelUiOp::AllModes(on) => self.prefs.settings.selection_all_modes = on,
        }
    }

    /// 多角形の点を打っている途中か（取り消しが最後の点に当たる間。メニューの「取り消し」もこのあいだは押せる）。
    pub fn sel_has_polygon_point(&self) -> bool {
        self.tool == Tool::Polygon
            && (!self.sel.polygon.is_empty() || !self.sel.view3d.polygon.is_empty())
    }

    /// 多角形の途中の点があれば最後の 1 つを取り消す（取り消しのキーを文書でなく途中の形に当てる）。取り消したか。
    pub fn sel_undo_polygon_point(&mut self) -> bool {
        if !self.sel_has_polygon_point() {
            return false;
        }
        // 3D ビューで打っている点を先に（2D の点と同時には無い）
        if !self.sel.view3d.remove_last_point() {
            canvas::remove_last_point(self);
        }
        true
    }

    /// 文書（セット）が替わったとき: 前の文書に向けた途中の形と、量を聞くウィンドウを捨てる。
    pub fn sel_doc_changed(&mut self) {
        self.sel.cancel_drafts();
        self.sel.dialog = None;
        // クイックマスクは今の文書の見え方。ストロークも重ね表示も、前の文書のものは持ち越さない
        self.sel.pen = None;
        self.sel.quick = false;
        self.sel.quick_overlay.clear();
        self.sel.pen_overlay.clear();
    }

    /// ツールを替えたとき: 途中の形を捨てる。
    pub fn sel_tool_changed(&mut self) {
        self.sel.cancel_drafts();
    }

    /// 選んでいるレイヤーで描くときの 2D の対称（効いている 2D の対称定規の写し。無ければ切）。2D のキャンバスのストロークにも、3D ビューの
    /// 面のストローク（UV の平面で、3D の写しの後に）にも渡す。
    pub fn canvas_symmetry(&self) -> CanvasSymmetry {
        self.canvas_symmetry_for(self.selected_layer)
    }

    /// `drawing` で描くときの 2D の対称。
    pub fn canvas_symmetry_for(&self, drawing: Option<crate::engine::LayerId>) -> CanvasSymmetry {
        self.rulers_active_for(crate::rulers::Place::Canvas, drawing)
            .canvas_symmetry
            .unwrap_or_default()
    }

    /// 2D のキャンバスのストロークに当てる 3D の対称（効いている 3D の対称定規があって、今のテクスチャセットの面のモデルがあるとき）。ダブの中心の UV の
    /// 下の面の点を 3D で写し、写しの面の UV へダブを置く（core の `ModelSymmetry`。UV の格子は範囲のツールと共有する）。
    pub fn canvas_model_symmetry(
        &mut self,
    ) -> Option<std::sync::Arc<yolu_core::geometry::ModelSymmetry>> {
        self.canvas_model_symmetry_for(self.selected_layer)
    }

    fn canvas_model_symmetry_for(
        &mut self,
        drawing: Option<crate::engine::LayerId>,
    ) -> Option<std::sync::Arc<yolu_core::geometry::ModelSymmetry>> {
        let setup = self
            .rulers_active_for(crate::rulers::Place::Canvas, drawing)
            .surface_symmetry?;
        let grid = self.region_grid()?;
        yolu_core::geometry::ModelSymmetry::new(grid, &setup).map(std::sync::Arc::new)
    }

    /// 幾何形状に沿って描く。補間と手ぶれ補正だけを切り、入り抜きと筆先の設定は保つ。
    pub fn begin_guided_canvas_stroke(
        &mut self,
        id: crate::engine::LayerId,
        eraser: bool,
        stencil: Option<std::sync::Arc<yolu_core::BrushStencil>>,
    ) -> Result<crate::engine::Stroke, CoreError> {
        let mut brush = self.stroke_brush(eraser);
        brush.stencil = stencil;
        brush.symmetry = self.canvas_symmetry_for(Some(id));
        brush.model_symmetry = self.canvas_model_symmetry_for(Some(id));
        brush.assist.stabilizer = 0.0;
        brush.assist.curve = false;
        let result = self.begin_stroke_with(id, &brush);
        self.sel.stroke_symmetry = result
            .is_ok()
            .then_some(brush.symmetry)
            .filter(|s| s.enabled());
        result
    }

    /// 2D のキャンバスで描き始める（2D の対称と 3D の対称を渡す。指先・クローンは対称と組めないので core が断る）。3D の面のストロークは
    /// `begin_paint_stroke`。
    pub fn begin_canvas_stroke(
        &mut self,
        id: crate::engine::LayerId,
        eraser: bool,
        stencil: Option<std::sync::Arc<yolu_core::BrushStencil>>,
    ) -> Result<crate::engine::Stroke, CoreError> {
        let mut brush = self.stroke_brush(eraser);
        brush.stencil = stencil;
        brush.symmetry = self.canvas_symmetry_for(Some(id));
        brush.model_symmetry = self.canvas_model_symmetry_for(Some(id));
        let result = self.begin_stroke_with(id, &brush);
        self.sel.stroke_symmetry = result
            .is_ok()
            .then_some(brush.symmetry)
            .filter(|s| s.enabled());
        result
    }
}
