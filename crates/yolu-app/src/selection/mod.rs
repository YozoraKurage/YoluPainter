//! 選択範囲と 2D の対称の道具（画面の側）。形（矩形・楕円・投げ縄・多角形・自動選択）を core の `SelectionMask` にして、今の選択範囲と
//! 作成方法（新規・追加・削除・共通）で組み合わせ、1 回の Undo で文書に置く。メニュー（すべて・解除・反転・拡張・縮小・境界・ぼかし・くっきり）も、
//! core の `SelectionMask` の操作を呼ぶだけ。選択範囲は文書（`Document`）が持つので、セットごとに別で、Undo・.ylp の保存と読み込みも文書に付く。
//!
//! - `canvas`: キャンバスの入力（ドラッグ・クリック・Esc）と、選択の縁（点線が流れる表示）・ドラッグ中の形・対称の軸の表示
//! - `outline`: 選択範囲の縁の線分（点線の元）
//! - `symmetry`: 2D の対称の設定（縦・横・両方・放射状）と軸、3D の面の対称（ミラー・放射状）
//! - `menu`・`props`・`dialog`: 選択メニュー・オプションバーとプロパティの欄・量を聞く小さな窓
//! - `io`: .ylp の `selection.bin` との受け渡し
//! - `pen`: 選択ペン・選択消し（ブラシで塗るように選択範囲を足す・消す）。`quick`: クイックマスク（選択範囲を赤い重ねで見せ、ブラシ・
//!   消しゴムで直す）。`overlay`: マスクの量を色つきの重ねで見せる。`saved`: 名前を付けて残した選択範囲（文書の持ち物。.ylp に保存）
//!
//! 文書を変える操作は `Action::Sel(SelAction::Edit(..))`（1 つが 1 回の Undo。描いている間と読むだけのセットでは断る）、画面だけの
//! 操作は `SelAction::Ui`・`SelAction::Symmetry`（Undo に入らない）。対称は文書に入れない画面の設定（2D と 3D は別々）で、ストロークを始めるときに
//! ブラシへ写して固める（途中で変えても、そのストロークには効かない）。

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
pub mod symmetry;

use crate::engine::{
    CanvasSymmetry, CoreError, DVec2, Document, LayerKind, SelectionCombine, SelectionMask,
    SymmetryMode, DEFAULT_WORKING_BUDGET_BYTES, MAX_MODIFY_RADIUS,
};
use crate::lang::Lang;
use crate::state::{Action, AppState, StrokeSource, Tool};

pub use self::symmetry::SymmetryState;

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

    /// 画布の縁を固定するかの設定が効くか（拡張は外へ広がるだけなので効かない）。
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
    /// 投げ縄・多角形（画布の座標の点。3 つ未満なら何も選ばない）。
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
    /// 自動選択（許し幅・隣接・全レイヤーは `SelState` の今の値）。
    Wand {
        x: u32,
        y: u32,
        mode: SelectionCombine,
    },
    /// 選択範囲を描画色で塗りつぶす（選んでいる層。マスクを描いているならマスク）。
    Fill,
    /// 選択範囲の画素を消す（アルファを減らす。マスクなら隠す）。
    Erase,
    /// 選択範囲の画素を新しいレイヤーとして元の位置にコピーする（クリップボードは変えない）。
    ToNewLayer,
    /// 選択範囲を選んでいる層のマスクにする（外を隠す）。
    ToMask,
}

/// 画面だけの選択の操作（Undo に入らない）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelUiOp {
    /// 選択の道具の組み合わせ方（オプションバー）。
    Combine(SelectionCombine),
    /// 量を聞く窓を開く（選択範囲が無ければ開かない）。
    OpenAmount(ModifyKind),
    /// 窓の値で適用して閉じる。
    ApplyAmount,
    CancelAmount,
    /// 選択範囲の下のボタンの帯を出す・出さない。
    Bar(bool),
    /// クイックマスクを入れる・切る（None は切り替え）。
    QuickMask(Option<bool>),
    /// 選択ペンの道具の基本（false が選択ペン、true が選択消し。Shift・Ctrl は押している間だけ替える）。
    PenErase(bool),
}

/// 2D の対称の設定の操作（画面だけ）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SymOp {
    Mode(SymmetryMode),
    /// 入っているなら切り、切っているなら最後のモードを入れ直す。
    Toggle,
    /// 放射状の写しの数（2〜16 に丸める）。
    Count(u32),
    /// 中心（文書の大きさに対する 0〜1）。
    Center(f64, f64),
    /// 中心をキャンバスの中央に戻す。
    CenterCanvas,
    ShowAxes(bool),
    /// 3D の面のミラー（軸に直交する面で左右に写す）。
    Mirror3d(bool),
    Axis3d(yolu_core::geometry::SymmetryAxis),
    /// 面のずれ（モデルの原点から、軸の向きに。モデルの単位）。
    Offset3d(f32),
    /// 面をモデルの原点・境界の中央に置く。
    OffsetOrigin3d,
    OffsetBounds3d,
    /// 3D の面の放射状（軸のまわりに回して写す）。
    Radial3d(bool),
    RadialAxis3d(yolu_core::geometry::SymmetryAxis),
    RadialCount3d(u32),
    /// 写しの側は見えない面にも塗る。
    IgnoreVisibility3d(bool),
    /// 3D ビューに対称の面を出す。
    ShowPlane3d(bool),
}

/// `Action::Sel` の中身。
#[derive(Clone, Debug, PartialEq)]
pub enum SelAction {
    Edit(SelEdit),
    Ui(SelUiOp),
    Symmetry(SymOp),
    /// 名前を付けて残した選択範囲（残す・名前を変える・消す・窓。文書の持ち物で、1 回の Undo。.ylp に保存）。
    Saved(saved::SavedOp),
}

/// 量を聞く窓の状態。
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
    /// 画布の座標。
    pub start: (f64, f64),
    pub current: (f64, f64),
    /// 投げ縄の点（1 画素以上離れたものだけ）。
    pub lasso: Vec<(f64, f64)>,
    /// 画面で動いた最大の距離（クリックかドラッグか）。
    pub moved: f32,
}

/// 選択範囲と対称の画面の状態。
pub struct SelState {
    /// 選択の道具の組み合わせ方（キーの修飾が無いとき）。
    pub combine: SelectionCombine,
    /// 自動選択の許し幅（0〜255）・隣接・全レイヤー（選んだレイヤーでなく合成から選ぶ）。
    pub tolerance: u8,
    pub contiguous: bool,
    pub all_layers: bool,
    /// 拡張・縮小・境界・ぼかしの半径（画素）と、画布の縁を固定するか。
    pub radius: u32,
    pub edge_lock: bool,
    pub dialog: Option<AmountDialog>,
    /// 窓を見出しで動かした量。
    pub dialog_offset: egui::Vec2,
    pub drag: Option<ShapeDrag>,
    /// 多角形の途中の点（画布の座標）と、ポインタの今の位置（ゴムの線）。
    pub polygon: Vec<(f64, f64)>,
    pub polygon_hover: Option<(f64, f64)>,
    /// 最後に押した時刻と点（ダブルクリックで多角形を閉じる）。
    pub last_press: Option<(f64, egui::Pos2)>,
    /// 形のドラッグを押し始めたときの修飾（離したときと見比べて、追加か縦横比の固定かを分ける）。
    pub press_modifiers: egui::Modifiers,
    /// ペンが触れている間の ID（ペンの触れる・離すを押す・離すにする）。
    pub pen_down: Option<u32>,
    pub symmetry: SymmetryState,
    /// 描いているストロークに固めた対称（軸の表示はこれ）。
    pub stroke_symmetry: Option<CanvasSymmetry>,
    /// 縁の滑らかさ（楕円・なげなわ・多角形・角丸の長方形）。切ると縁は 0 か 255 だけ。
    pub antialias: bool,
    /// 長方形・楕円を常に縦横比 1:1（正方形・正円）にする（Shift を押しているあいだの形を、押さなくても）。
    pub fixed_ratio: bool,
    /// 長方形・楕円を、押した点が中心になるように広げる（Alt を押しているあいだの形を、押さなくても）。
    pub from_center: bool,
    /// 長方形の角の丸め（画素。0 は丸めない）。
    pub corner_radius: u32,
    /// 選択ペンの道具の基本: false が選択ペン、true が選択消し。
    pub pen_erase: bool,
    /// 動いているペンのストローク（選択ペンの道具・クイックマスクのブラシ）。
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
    /// 残した選択範囲の窓（開いていれば）。
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
            radius: 5,
            edge_lock: false,
            dialog: None,
            dialog_offset: egui::Vec2::ZERO,
            drag: None,
            polygon: Vec::new(),
            polygon_hover: None,
            last_press: None,
            press_modifiers: egui::Modifiers::NONE,
            pen_down: None,
            symmetry: SymmetryState::default(),
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
        // クイックマスクのブラシのストロークは描くストロークの流れ（`canvas.stroke`）が終わらせる。選択ペンの道具のものは、ほかの形と同じく捨てる
        let pen = self.pen.as_ref().is_some_and(|a| !a.quick);
        if pen {
            self.pen = None;
        }
        let any = self.drag.is_some() || !self.polygon.is_empty() || pen;
        self.drag = None;
        self.polygon.clear();
        self.polygon_hover = None;
        self.last_press = None;
        self.pen_down = None;
        any
    }

    /// 選択範囲に量のある画素を全部含む矩形（x0, y0, x1, y1。半開区間、画布の画素の座標）。何も選んでいなければ None。
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

/// 組み合わせ方のツールチップ（キー付き）。
pub fn combine_tooltip(lang: Lang, mode: SelectionCombine) -> &'static str {
    match mode {
        SelectionCombine::Replace => lang.pick(
            "新規選択: 新しい形で置き換える",
            "New: replace the selection",
        ),
        SelectionCombine::Add => lang.pick(
            "追加選択: 選択範囲に足す（Shift）",
            "Add to the selection (Shift)",
        ),
        SelectionCombine::Subtract => lang.pick(
            "一部削除: 選択範囲から引く（Ctrl）",
            "Subtract from the selection (Ctrl)",
        ),
        SelectionCombine::Intersect => lang.pick(
            "選択中を選択: 重なる所だけ残す（Shift + Ctrl）",
            "Intersect: keep only the overlap (Shift + Ctrl)",
        ),
    }
}

/// キーの修飾から組み合わせ方（Shift で足す・Ctrl で引く・両方で重ねる。無ければオプションバーの値。組み合わせは `keymap::GESTURES` の表）。
pub fn combine_of(base: SelectionCombine, modifiers: egui::Modifiers) -> SelectionCombine {
    use crate::keymap::Operation;
    match crate::keymap::gesture("selection", egui::PointerButton::Primary, &modifiers, false) {
        Some(Operation::SelectionAdd) => SelectionCombine::Add,
        Some(Operation::SelectionSubtract) => SelectionCombine::Subtract,
        Some(Operation::SelectionIntersect) => SelectionCombine::Intersect,
        _ => base,
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
            SelAction::Symmetry(op) => self.sel_symmetry(op),
            SelAction::Saved(op) => self.sel_saved(op),
        }
    }

    /// 選択範囲を変える（描いている間と読むだけのセットは `Action::apply` が先に断る）。断られたら何も変えず、理由をステータスバーへ。
    pub fn sel_edit(&mut self, edit: SelEdit) {
        if self.is_stroking() {
            self.message = self
                .lang
                .pick("描いている間はできません。", "Not while drawing.")
                .into();
            return;
        }
        let revision = self.doc.revision();
        match self.sel_apply(edit) {
            Ok(text) => self.message = text,
            Err(e) => self.message = e,
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
    fn edge_of(&self, shape: SelectionMask) -> SelectionMask {
        if self.sel.antialias {
            shape
        } else {
            shape.sharpen()
        }
    }

    /// 自動選択の基準の層（選んだレイヤー。ラスターでない・全レイヤーなら None で、チャンネルの合成）。
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
            SelEdit::Wand { x, y, mode } => {
                let shape = SelectionMask::magic_wand(
                    &self.doc,
                    self.wand_layer(),
                    self.m2.paint_channel,
                    x,
                    y,
                    self.sel.tolerance,
                    self.sel.contiguous,
                    DEFAULT_WORKING_BUDGET_BYTES,
                )
                .map_err(|e| lang.core_error(&e))?;
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
                    self.message = self
                        .lang
                        .pick("描いている間はできません。", "Not while drawing.")
                        .into();
                } else if self.doc.selection().is_none() {
                    self.message = self
                        .lang
                        .pick("選択範囲がありません。", "No selection.")
                        .into();
                } else if let Some(reason) = self.read_only_reason() {
                    self.message = format!(
                        "{}: {reason}",
                        self.lang
                            .pick("読むだけのテクスチャセットです", "Read-only texture set")
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
        }
    }

    /// 2D の対称の設定の操作（画面だけ。描いている間は軸の表示のほかは断る: ストロークに固めた設定と食い違わせない）。
    pub fn sel_symmetry(&mut self, op: SymOp) {
        if self.is_stroking() && !matches!(op, SymOp::ShowAxes(_) | SymOp::ShowPlane3d(_)) {
            self.message = self
                .lang
                .pick("描いている間はできません。", "Not while drawing.")
                .into();
            return;
        }
        // 境界の中央は、モデルの今の形から（モデルが無ければ何もしない）
        let bounds_center = self
            .view3d
            .model
            .as_ref()
            .map(|m| m.geometry.bounds().center);
        let s = &mut self.sel.symmetry;
        match op {
            SymOp::Mode(mode) => s.set_mode(mode),
            SymOp::Toggle => s.toggle(),
            SymOp::Count(n) => s.set_count(n),
            SymOp::Center(x, y) => s.set_center(x, y),
            SymOp::CenterCanvas => s.center = (0.5, 0.5),
            SymOp::ShowAxes(on) => s.show_axes = on,
            SymOp::Mirror3d(on) => s.surface.mirror = on,
            SymOp::Axis3d(axis) => s.surface.axis = axis,
            SymOp::Offset3d(v) => s.surface.set_offset(v),
            SymOp::OffsetOrigin3d => s.surface.offset = 0.0,
            SymOp::OffsetBounds3d => {
                if let Some(center) = bounds_center {
                    let offset = s.surface.bounds_center_offset(center);
                    s.surface.set_offset(offset);
                }
            }
            SymOp::Radial3d(on) => s.surface.radial = on,
            SymOp::RadialAxis3d(axis) => s.surface.radial_axis = axis,
            SymOp::RadialCount3d(n) => s.surface.set_radial_count(n),
            SymOp::IgnoreVisibility3d(on) => s.surface.ignore_visibility = on,
            SymOp::ShowPlane3d(on) => s.surface.show_plane = on,
        }
    }

    /// 多角形の点を打っている途中か（取り消しが最後の点に当たる間。メニューの「取り消し」もこのあいだは押せる）。
    pub fn sel_has_polygon_point(&self) -> bool {
        self.tool == Tool::Polygon && !self.sel.polygon.is_empty()
    }

    /// 多角形の途中の点があれば最後の 1 つを取り消す（取り消しのキーを文書でなく途中の形に当てる）。取り消したか。
    pub fn sel_undo_polygon_point(&mut self) -> bool {
        if !self.sel_has_polygon_point() {
            return false;
        }
        canvas::remove_last_point(self);
        true
    }

    /// 文書（セット）が替わったとき: 前の文書に向けた途中の形と、量を聞く窓を捨てる。
    pub fn sel_doc_changed(&mut self) {
        self.sel.cancel_drafts();
        self.sel.dialog = None;
        // クイックマスクは今の文書の見え方。ストロークも重ね表示も、前の文書のものは持ち越さない
        self.sel.pen = None;
        self.sel.quick = false;
        self.sel.quick_overlay.clear();
        self.sel.pen_overlay.clear();
    }

    /// 道具を替えたとき: 途中の形を捨てる。
    pub fn sel_tool_changed(&mut self) {
        self.sel.cancel_drafts();
    }

    /// 2D のキャンバスのストロークに渡す対称（文書の大きさに写した中心）。3D の面のストロークには渡さない（core は見ない）。
    pub fn canvas_symmetry(&self) -> CanvasSymmetry {
        self.sel
            .symmetry
            .canvas(self.doc.width(), self.doc.height())
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
        brush.symmetry = self.canvas_symmetry();
        brush.assist.stabilizer = 0.0;
        brush.assist.curve = false;
        let result = self.begin_stroke_with(id, &brush);
        self.sel.stroke_symmetry = result
            .is_ok()
            .then_some(brush.symmetry)
            .filter(|s| s.enabled());
        result
    }

    /// 2D のキャンバスで描き始める（対称を渡す。指先・クローンは対称と組めないので core が断る）。3D の面のストロークは `begin_paint_stroke`。
    pub fn begin_canvas_stroke(
        &mut self,
        id: crate::engine::LayerId,
        eraser: bool,
        stencil: Option<std::sync::Arc<yolu_core::BrushStencil>>,
    ) -> Result<crate::engine::Stroke, CoreError> {
        let mut brush = self.stroke_brush(eraser);
        brush.stencil = stencil;
        brush.symmetry = self.canvas_symmetry();
        let result = self.begin_stroke_with(id, &brush);
        self.sel.stroke_symmetry = result
            .is_ok()
            .then_some(brush.symmetry)
            .filter(|s| s.enabled());
        result
    }
}

/// 文書の選択範囲があるか。
pub fn has_selection(doc: &Document) -> bool {
    doc.selection().is_some()
}
