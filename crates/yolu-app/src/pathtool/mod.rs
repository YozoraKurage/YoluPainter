//! パスのツール（P）: 2D のキャンバスと 3D のモデルの面の上に、編集できるパス（制御点を通る曲線）を引く。レイヤーはパスから描いた画素を持ち、
//! パスと画素はいつも一緒に変わる（点を足す・差し込む・動かす・消す・閉じる・開く・太さを変える・ブラシや組を変える、どれも描き直して
//! 1 回の Undo）。パスレイヤーには手で描けず、ラスタライズでパスを外すと今の画素だけが残る。
//!
//! - 2D のパスは画素の座標に、3D のパスはモデルの三角形と重心座標に結び付く（指紋が違うモデルでは編集せず、`rebind` で付け直す）。
//!   1 つのレイヤーはどちらか一方だけを持つ。曲線は core が点を順に通る centripetal Catmull-Rom で描き、`curve` が同じ式で画面に見せる。
//! - 点の操作は `edit`（点の並びだけの純粋な関数）、入力と重ね表示は 2D が `canvas`、3D が `surface`。入力は
//!   ストロークと同じ道（押す・動く・離す・Esc・フォーカスを失う・取りこぼし）から呼び、点のドラッグは離したとき 1 回で当てる。
//! - 組（マテリアル）がオンなら、パスは組のチャンネルの全部を 1 回で描く。
//! - 3D の点の三角形の番号は、隠したマテリアルを除いた「見せる形」ではなく「受けたままの形」の番号（保存と Unity 版と同じ）。

pub mod canvas;
pub mod curve;
pub mod edit;
pub mod list;
pub mod presets;
pub mod rebind;
pub mod surface;

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::sync::{Arc, Mutex};

use egui::{Color32, Pos2};
use yolu_core::paths::{
    self, render_canvas, render_list, render_surface, CanvasPath, ChannelPaint, LayerPathEntry,
    Options, PathBrush, PathKind, PathStyle, SurfacePath,
};
use yolu_core::{CoreError, LayerId, LayerPath};

use self::edit::{Place, PointOp};
use crate::lang::Lang;
use crate::notice::Source;
use crate::state::{AppState, StrokeSource};
use crate::view3d::model::ViewModel;

/// 点を掴む距離・線に近いとみなす距離（画面の点）。
pub const GRAB_RADIUS: f32 = 8.0;
/// これ以内（画面の点）しか動かさなければ、ドラッグでなくクリック（点を選ぶだけ）。
pub const CLICK_RADIUS: f32 = 4.0;
/// パスの線と点の色（Unity 版と同じ橙）。
pub const PATH_COLOR: Color32 = Color32::from_rgb(255, 204, 51);

/// 選んでいる点（レイヤーとパスの ID で確かめる。レイヤー・パスが替わったら無効）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PointRef {
    pub layer: LayerId,
    pub path: u128,
    pub index: usize,
}

/// 取っ手の側。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HandleSide {
    /// 前の区間からその点へ入る側。
    In,
    /// その点から次の区間へ出る側。
    Out,
}

/// ポインタの下に何があるか。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Hover {
    None,
    /// 選んでいる点の取っ手の先（点の番号と側）。
    Handle(usize, HandleSide),
    /// 点（番号）。
    Point(usize),
    /// 曲線の区間（番号）と、そこでポインタにいちばん近い画面の点（差し込む位置の印）。
    Segment(usize, Pos2),
}

/// 点のドラッグ（離したとき 1 回で `Move` を当てる。途中は重ね表示だけが動く）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PointDrag {
    pub layer: LayerId,
    pub path: u128,
    pub index: usize,
    pub source: StrokeSource,
    /// 3D のパスの点か。
    pub surface: bool,
    pub start: Pos2,
    /// 押した所からいちばん離れた距離（画面の点）。
    pub moved: f32,
    /// 今の置き場所（2D はキャンバスの中に収めた画素、3D はポインタの下の面。置けない所では前のまま）。
    pub target: Option<Place>,
    /// 取っ手のドラッグなら、その側（点そのものを動かすなら None）。
    pub handle: Option<HandleSide>,
    /// 取っ手の今の向き（点から先まで。2D は画素、3D は休みの形のモデルの空間）。
    pub vector: Option<[f64; 3]>,
}

/// 点を矩形で選ぶドラッグ（画面の点）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RectDrag {
    pub source: StrokeSource,
    /// 3D のビューか。
    pub surface: bool,
    pub start: Pos2,
    pub now: Pos2,
}

/// 入力の修飾キーと時刻（ビューが入力のたびに入れる）。取っ手のドラッグ（Alt で折る・Ctrl で両方を伸ばす）と、点のダブルクリック
/// （角と滑らかの切り替え）に使う。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PathInputState {
    pub alt: bool,
    pub ctrl: bool,
    /// Shift: 押した所からの矩形で点を選ぶ。
    pub shift: bool,
    /// 入力の時刻（秒）。None ならダブルクリックを見ない（画面の無い試験）。
    pub now: Option<f64>,
}

/// ダブルクリックとみなす間（秒）。
pub const DOUBLE_CLICK: f64 = 0.4;

/// ペンが触れている間のペンの番号と、触れたビュー（2D のキャンバスと 3D のビューが並んで見えていても、離したのを受け取るのは
/// 触れたビューだけ。相手のビューが離したサンプルで `pen_down` を下ろすと、触れたビューのドラッグが取り残される）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PenDown {
    pub id: u32,
    /// 3D のビューが触れたか（false なら 2D のキャンバス）。
    pub surface: bool,
}

/// 編集しているパス（レイヤーと、そのレイヤーの一覧のパスの ID）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ActivePath {
    pub layer: LayerId,
    pub path: u128,
}

/// パスのツールの状態（パスそのものは文書が持つ）。
#[derive(Default)]
pub struct PathState {
    pub selected: Option<PointRef>,
    /// 一覧で選んだパス（無い・レイヤーが替わったら、レイヤーの一覧のいちばん上のパス）。
    pub active: Option<ActivePath>,
    /// 矩形・全部で選んだ点（`selected` の点と一緒に、太さ・角・取っ手・消すを当てる）。パスが替わったら効かない。
    pub marked: Option<(ActivePath, Vec<usize>)>,
    /// 点を矩形で選んでいるドラッグ（Shift を押して押した所から）。
    pub rect: Option<RectDrag>,
    /// パスの編集を抜けたレイヤー（Esc・Enter・「新しいパス」）。次に置く点は、そのレイヤーの一覧に新しいパスを始める。
    pub fresh: Option<LayerId>,
    /// 写したパス（貼り付け・設定や位置の貼り付けに使う）。
    pub clipboard: Option<LayerPathEntry>,
    /// 名前を変えている一覧の行（レイヤーとパスの ID）。
    pub renaming: Option<ActivePath>,
    /// 名前の入力欄を開いたあとのフレームか（開いたフレームだけ入力欄に移る）。
    pub rename_started: bool,
    /// 修飾キーと時刻。
    pub input: PathInputState,
    /// 次に作るパスの描き方（パスを選んでいないときの種類・筆先・深さの欄）。
    pub next_style: PathStyle,
    /// パスのプリセット（設定のフォルダの `path_presets/`）。
    pub presets: presets::PathPresets,
    /// 名前を変えているプリセット（プリセットのボタンの所に入力欄）。
    pub preset_renaming: Option<u32>,
    /// プリセットの名前の入力欄を開いたあとのフレームか。
    pub preset_rename_started: bool,
    /// 最後に点を押した時刻と、その点（ダブルクリックを見る）。
    pub last_press: Option<(f64, PointRef)>,
    pub drag: Option<PointDrag>,
    /// ペンが触れている間のペンの番号と触れたビュー。
    pub pen_down: Option<PenDown>,
    /// スライダーを動かしている間の値（キー・値）。離したとき 1 回で当てる。
    pub pending: Option<(&'static str, f32)>,
    /// Esc を扱ったフレーム（2D と 3D の両方が見えていて、同じ Esc を 2 回扱わないため）。
    esc_frame: Option<u64>,
    /// パスの欄の名前の入力欄が入力を受けていた最後のフレーム（その Esc は入力をやめるのに使い、点の選びやパスの編集には効かせない）。
    typing_frame: Option<u64>,
    /// 3D の指紋（ポインタ・世代で覚える。三角形の数だけかかるので毎回は求めない）。
    fingerprint: Mutex<Option<(usize, u32, Arc<str>)>>,
}

impl PathState {
    /// パスの欄の名前の入力欄が、このフレーム（`frame`）に入力を受けている。
    pub fn note_typing(&mut self, frame: u64) {
        self.typing_frame = Some(frame);
    }

    /// ペンが、この種類のビュー（`surface` が 3D か）に触れているか。
    pub fn pen_in(&self, surface: bool) -> bool {
        self.pen_down.is_some_and(|p| p.surface == surface)
    }
}

/// ブラシの値の操作。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BrushEdit {
    /// 直径（画素。3D は今のモデルでの同じ大きさ）。
    Diameter(f64),
    Hardness(f64),
    Spacing(f64),
    Opacity(f64),
    Flow(f64),
    PressureSize(bool),
    PressureOpacity(bool),
    PressureFlow(bool),
    /// 縁のアンチエイリアス。
    AntiAlias(yolu_core::AntiAlias),
}

/// パスの操作（メニュー・キー・ボタン・試験が同じ道を通る）。
#[derive(Clone, Debug, PartialEq)]
pub enum PathAction {
    /// 選んでいる点を替える（None で外す）。文書は変えない。
    Select(Option<usize>),
    /// 点の操作。選んでいるレイヤーにパスが無く、足す操作なら、その上に新しいパスレイヤーを作る。
    Point(PointOp),
    /// 選んでいる点（無ければ最後の点）を消す。
    DeleteSelected,
    /// パスのブラシの値（パスが無いときは、次に作るパスが取る今のブラシ）。
    Brush(BrushEdit),
    /// 今のブラシ・マテリアルで塗るチャンネルの組をパスに使う。
    UseBrush,
    /// 今のモデルで描き直す（休みの形で描くので、ポーズによらない）。
    Redraw,
    /// パスを外して今の画素だけを残す。
    Rasterize(LayerId),
    /// 一覧のパスを選ぶ（None でパスの編集を抜け、次の点は新しいパスを始める）。文書は変えない。
    SelectPath(Option<u128>),
    /// 一覧の操作（文書を変えるものは 1 つが 1 回の Undo）。
    List(list::ListOp),
    /// 一覧の行の名前を変え始める（入力欄を開く。文書は変えない）。
    BeginRename(u128),
    /// 選んでいる点を角にする（false で滑らかに戻す）。
    SetCorner(bool),
    /// 選んでいる点に取っ手を出す（今の曲がりのままの取っ手。false で滑らかに戻す）。
    SetHandles(bool),
    /// パスの種類（リボンの画像・並べ方・間隔、指先の強さを含む）。パスが無いときは、次に作るパスの種類。
    Kind(PathKind),
    /// 筆先・角度・向き・投影の深さ。パスが無いときは、次に作るパスの設定。
    Style(StyleEdit),
    /// パスの対称（入れると、効いている対称定規の値を写して、映したパスも描く。2D は 2D の対称定規、3D は鏡の面 1 枚の 3D の対称定規。無ければ断る）。
    Symmetry(bool),
    /// パスの向きを逆にする（点の並びを逆に）。
    Reverse,
    /// パスの全部の点を選ぶ。
    SelectAllPoints,
    /// 選んでいる点の全部の太さ（0〜1。1 回の Undo）。
    Widths(f64),
    /// プリセット（保存・当てる・名前の変更・消す）。
    Preset(presets::PresetOp),
}

/// 筆先と投影の深さの値の操作。
#[derive(Clone, Debug, PartialEq)]
pub enum StyleEdit {
    /// 筆先の画像（None は丸）。
    Tip(Option<Arc<yolu_core::BrushTip>>),
    /// 筆先の角度（度）。
    Angle(f64),
    /// 筆先をパスの進む向きに回す。
    Follow(bool),
    /// 3D の投影の深さ（ブラシの半径の倍数。None は自動）。
    Depth(Option<f64>),
}

impl PathAction {
    /// 文書を変える操作か（読むだけのセットでは断る）。
    pub fn edits_document(&self) -> bool {
        match self {
            PathAction::Select(_)
            | PathAction::SelectPath(_)
            | PathAction::BeginRename(_)
            | PathAction::SelectAllPoints => false,
            PathAction::List(op) => op.edits_document(),
            // 保存・名前の変更・消すは設定のフォルダだけ（当てるのは文書を変える）
            PathAction::Preset(op) => matches!(op, presets::PresetOp::Apply(_)),
            _ => true,
        }
    }
}

/// 3D の文脈（受けたままのモデル・その指紋・描くテクスチャセットのマテリアル・描く形）。
pub struct SurfaceCtx {
    /// 編集する形（点を置く・動かす・重ね表示。ポーズを付けた形）。
    pub model: Arc<ViewModel>,
    pub fingerprint: Arc<str>,
    pub material: i32,
    /// 描く形（休みの形）。ポーズを変えてもテクスチャのパスの絵が変わらないよう、描く・描き直す・ブラシの大きさの換算はこの形で。
    pub render: Arc<yolu_core::geometry::SurfaceGeometry>,
}

/// 128 bit の新しい ID。
fn random_id() -> u128 {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let state = RandomState::new();
    let mut a = state.build_hasher();
    a.write_u64(n);
    let mut b = state.build_hasher();
    b.write_u64(!n);
    b.write_u8(0x50);
    ((a.finish() as u128) << 64) | b.finish() as u128
}

/// 点が 1 つも無い 1 本だけの一覧か（置く側が、まだ決まっていない）。
fn is_unplaced(entries: &[LayerPathEntry]) -> bool {
    matches!(entries, [only] if only.path.point_count() == 0)
}

/// パスを持つレイヤーの名前。
fn layer_name(lang: Lang, n: usize) -> String {
    format!("{} {n}", lang.pick("パス", "Path"))
}

/// パスの描き方の設定を替えた新しいパス。
pub fn with_style(path: &LayerPath, style: PathStyle) -> LayerPath {
    match path {
        LayerPath::Canvas(c) => LayerPath::Canvas(CanvasPath { style, ..c.clone() }),
        LayerPath::Surface(s) => LayerPath::Surface(SurfacePath { style, ..s.clone() }),
    }
}

/// パスのブラシを替えた新しいパス。
pub fn with_brush(path: &LayerPath, brush: PathBrush) -> LayerPath {
    match path {
        LayerPath::Canvas(c) => LayerPath::Canvas(CanvasPath { brush, ..c.clone() }),
        LayerPath::Surface(s) => LayerPath::Surface(SurfacePath { brush, ..s.clone() }),
    }
}

/// パスの組を替えた新しいパス。
pub fn with_material(path: &LayerPath, material: Option<Vec<ChannelPaint>>) -> LayerPath {
    match path {
        LayerPath::Canvas(c) => LayerPath::Canvas(CanvasPath {
            material,
            ..c.clone()
        }),
        LayerPath::Surface(s) => LayerPath::Surface(SurfacePath {
            material,
            ..s.clone()
        }),
    }
}

pub fn path_brush(path: &LayerPath) -> PathBrush {
    match path {
        LayerPath::Canvas(c) => c.brush,
        LayerPath::Surface(s) => s.brush,
    }
}

fn to_path_paints(paints: Vec<yolu_core::material::ChannelPaint>) -> Vec<ChannelPaint> {
    paints
        .into_iter()
        .map(|p| ChannelPaint {
            channel: p.channel,
            color: p.value,
        })
        .collect()
}

impl AppState {
    /// 選んでいるレイヤーと、編集しているパス（無い・編集を抜けたときは None）。
    pub fn path_layer(&self) -> Option<(LayerId, &LayerPath)> {
        let (layer, index) = self.path_active_index()?;
        Some((layer, &self.doc.layer(layer)?.paths().get(index)?.path))
    }

    /// レイヤーのパスを画素にできるか（パスがあり、塗りつぶしレイヤーでない。塗りつぶしレイヤーは画素を持たず、パスを外すと絵が消えるので、
    /// core が断る）。ボタン・メニューの有効の条件。
    pub fn path_can_rasterize(&self, id: LayerId) -> bool {
        self.doc
            .layer(id)
            .is_some_and(|l| l.has_paths() && l.kind() != yolu_core::LayerKind::Fill)
    }

    /// 選んでいるレイヤーと、編集しているパスの一覧の中の番号（下から）。一覧で選んだパスが無ければ、一覧のいちばん上のパス。
    /// 編集を抜けたレイヤー（`fresh`）では None（次の点は新しいパスを始める）。
    pub fn path_active_index(&self) -> Option<(LayerId, usize)> {
        let layer = self.selected_layer?;
        let entries = self.doc.layer(layer)?.paths();
        if entries.is_empty() {
            return None;
        }
        if let Some(a) = self.path.active.filter(|a| a.layer == layer) {
            if let Some(i) = entries.iter().position(|e| e.id() == a.path) {
                return Some((layer, i));
            }
        }
        if self.path.fresh == Some(layer) {
            return None;
        }
        Some((layer, entries.len() - 1))
    }

    /// 選んでいるレイヤーのパスの一覧（無ければ空）。
    pub fn path_entries(&self) -> &[LayerPathEntry] {
        self.selected_layer
            .and_then(|id| self.doc.layer(id))
            .map_or(&[], |l| l.paths())
    }

    /// 選んでいる点の番号（レイヤー・パスが替わっていたり、点が無くなっていたら None）。
    pub fn path_selected_index(&self) -> Option<usize> {
        let r = self.path.selected?;
        let (layer, path) = self.path_layer()?;
        (r.layer == layer && r.path == path.id() && r.index < path.point_count()).then_some(r.index)
    }

    /// 選んでいる点の番号の全部（矩形・全部で選んだ点と、選んでいる点。昇順・重なりなし）。
    pub fn path_selected_indices(&self) -> Vec<usize> {
        let Some((layer, path)) = self.path_layer() else {
            return Vec::new();
        };
        let mut out: Vec<usize> = match &self.path.marked {
            Some((a, v)) if a.layer == layer && a.path == path.id() => v
                .iter()
                .copied()
                .filter(|i| *i < path.point_count())
                .collect(),
            _ => Vec::new(),
        };
        out.extend(self.path_selected_index());
        out.sort_unstable();
        out.dedup();
        out
    }

    /// 選んだ点の全部に点の操作を順に当て、1 回で描き直す（消すときは後ろの点から）。
    fn path_points_many(&mut self, ops: Vec<PointOp>) {
        let lang = self.lang;
        let Some((layer, Some(path))) = self.path_target(None) else {
            return;
        };
        let mut next = path.clone();
        let mut select = self.path_selected_index();
        for op in &ops {
            match edit::apply(&next, op) {
                Ok((p, s)) => {
                    next = p;
                    if !matches!(op, PointOp::Remove(_)) {
                        select = s;
                    } else {
                        select = None;
                    }
                }
                Err(r) => {
                    self.refuse(Source::Path, crate::lang::refusals::path_edit(lang, r));
                    return;
                }
            }
        }
        if next == path {
            return;
        }
        if ops.iter().any(|o| matches!(o, PointOp::Remove(_))) {
            self.path.marked = None;
        }
        self.path_commit(Some(layer), next, select);
    }

    /// パスレイヤーが描くときの予算（文書のもの）と、リボンが読むアセットの画像（文書へ渡してある入力）。
    fn path_options(&self) -> Options<'static> {
        Options {
            width: self.doc.width(),
            height: self.doc.height(),
            tile_size: self.doc.tile_size(),
            source_budget_bytes: self.doc.source_budget_bytes(),
            stroke_budget_bytes: self.doc.stroke_budget_bytes(),
            images: self.doc.effect_inputs().images().clone(),
            ..Options::default()
        }
    }

    /// 3D のモデルの指紋（形が替わるまで作り直さない）。
    pub fn path_fingerprint(
        &self,
        geometry: &Arc<yolu_core::geometry::SurfaceGeometry>,
    ) -> Arc<str> {
        let key = (Arc::as_ptr(geometry) as usize, geometry.revision());
        let mut cache = self
            .path
            .fingerprint
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some((p, r, f)) = &*cache {
            if (*p, *r) == key {
                return f.clone();
            }
        }
        let f: Arc<str> = paths::fingerprint(geometry).into();
        *cache = Some((key.0, key.1, f.clone()));
        f
    }

    /// 3D のパスの文脈。モデルが無い・今のテクスチャセットがモデルに無いときは、その理由。
    pub fn path_surface_ctx(&self) -> Result<SurfaceCtx, String> {
        let Some(model) = self.view3d.full_model().cloned() else {
            return Err(self.lang.pick("モデルがありません", "No model").into());
        };
        if self.view3d.material < 0 {
            return Err(self.region_missing_reason());
        }
        Ok(SurfaceCtx {
            fingerprint: self.path_fingerprint(&model.geometry),
            material: self.view3d.material,
            render: model.rest_geometry().clone(),
            model,
        })
    }

    /// 次に作るパスの描くチャンネルの組（マテリアルで塗るがオンのとき。オフなら描くチャンネル 1 つ）。
    fn path_new_material(&self) -> Option<Vec<ChannelPaint>> {
        self.paints_material()
            .then(|| to_path_paints(self.paint_channels()))
    }

    /// 今のブラシをパスの筆にする。3D は半径をモデルの大きさに合わせる（Unity 版と同じ式）。
    fn path_new_brush(&self, ctx: Option<&SurfaceCtx>) -> PathBrush {
        let b = self.brush.settings(self.color.main, false);
        let radius = match ctx {
            None => b.radius.max(0.01),
            Some(ctx) => {
                let r = yolu_core::geometry::world_radius(
                    &ctx.render,
                    self.brush.radius as f64,
                    self.doc.width(),
                );
                (r as f64).max(0.000001)
            }
        };
        // 画面の間隔は f32 なので、下限の 1% は core の範囲（0.01 以上）をわずかに下回って渡る
        PathBrush(yolu_core::BrushSettings {
            radius,
            spacing: b.spacing.max(0.01),
            ..b
        })
    }

    /// 点の無い 1 本のパスを持つパスレイヤーを、選んでいるレイヤーのすぐ上（グループの中ならその中）に作り、選んで、パスのツールへ替える（1 回の Undo。
    /// 最初の点はツールで置く。置いた側（2D・3D）がそのパスの側になる）。描けない状態（描いている最中・読むだけのセット）では断る。
    pub fn new_path_layer(&mut self) {
        let lang = self.lang;
        if self.is_stroking() {
            self.refuse(Source::Path, crate::lang::refusals::during_stroke(lang));
            return;
        }
        if let Some(reason) = self.read_only_reason().map(str::to_owned) {
            self.refuse(
                Source::Path,
                crate::lang::refusals::read_only_set(lang, &reason),
            );
            return;
        }
        let path = CanvasPath {
            style: self.path.next_style.clone(),
            id: random_id(),
            channel: self.m2.paint_channel,
            brush: self.path_new_brush(None),
            points: Vec::new(),
            material: self.path_new_material(),
        };
        let rendered = match render_canvas(&path, &self.path_options()) {
            Ok(r) => r,
            Err(e) => {
                self.fail(Source::Path, crate::lang::path_error(lang, &e));
                return;
            }
        };
        let n = self.doc.layers().iter().filter(|l| l.has_paths()).count() + 1;
        let above = self
            .selected_layer
            .filter(|id| self.doc.layer(*id).is_some());
        let id = path.id;
        match self.doc.add_path_layer(
            &layer_name(lang, n),
            LayerPath::Canvas(path),
            rendered.channels,
            above,
        ) {
            Ok(layer) => {
                self.modified = true;
                self.select_new(layer);
                // ツールを替えると、ほかのツールの途中の状態を捨てる。編集するパスは、そのあとで決める
                self.switch_tool(crate::state::Tool::Path, false);
                self.path.active = Some(ActivePath { layer, path: id });
                self.path.fresh = None;
                self.path.selected = None;
                self.path.marked = None;
            }
            Err(e) => self.notify(
                crate::notice::Kind::of_core(&e),
                Source::Path,
                lang.core_error(&e),
            ),
        }
    }

    /// 点を 1 つ持つ新しいパス（今のブラシ・描くチャンネル・組で）。`channel` はレイヤーの一覧の基準のチャンネル（一覧に加えるとき。
    /// 一覧のパスはどれも同じ基準のチャンネル）。
    fn path_new(
        &self,
        place: Place,
        channel: Option<yolu_core::Channel>,
    ) -> Result<LayerPath, String> {
        let (id, channel, material) = (
            random_id(),
            channel.unwrap_or(self.m2.paint_channel),
            self.path_new_material(),
        );
        let style = self.path.next_style.clone();
        let empty = match place {
            Place::Canvas { .. } => LayerPath::Canvas(CanvasPath {
                style: style.clone(),
                id,
                channel,
                brush: self.path_new_brush(None),
                points: Vec::new(),
                material,
            }),
            Place::Surface { .. } => {
                let ctx = self.path_surface_ctx()?;
                LayerPath::Surface(SurfacePath {
                    style,
                    id,
                    channel,
                    brush: self.path_new_brush(Some(&ctx)),
                    points: Vec::new(),
                    model_fingerprint: ctx.fingerprint.to_string(),
                    material,
                })
            }
        };
        edit::apply(&empty, &PointOp::Add(place))
            .map(|(p, _)| p)
            .map_err(|r| crate::lang::refusals::path_edit(self.lang, r))
    }

    /// 編集する前の確かめ: 描ける状態か、選んでいるレイヤーとそのパス。`surface` は編集するのが 3D のパスか（None なら、あるパスのまま）。
    fn path_target(&mut self, surface: Option<bool>) -> Option<(LayerId, Option<LayerPath>)> {
        let lang = self.lang;
        if self.is_stroking() {
            self.refuse(Source::Path, crate::lang::refusals::during_stroke(lang));
            return None;
        }
        if let Some(reason) = self.read_only_reason().map(str::to_owned) {
            self.refuse(
                Source::Path,
                crate::lang::refusals::read_only_set(lang, &reason),
            );
            return None;
        }
        if self.m2.edit_mask {
            self.refuse(
                Source::Path,
                lang.pick(
                    "パスはマスクに描けません",
                    "A path cannot be drawn on a mask",
                ),
            );
            return None;
        }
        let Some(layer) = self
            .selected_layer
            .filter(|id| self.doc.layer(*id).is_some())
        else {
            self.refuse(
                Source::Path,
                lang.pick("描くレイヤーがありません。", "No layer to paint on."),
            );
            return None;
        };
        let existing = self.path_layer().map(|(_, p)| p.clone());
        // 一覧のパスはどれも同じ側（2D か 3D）。編集を抜けて新しいパスを始めるときも、レイヤーの一覧の側で断る。
        // 点が 1 つも無い 1 本だけのレイヤー（新規パスレイヤー・点を全部消したレイヤー）は、まだ側が決まっていない
        let side = self
            .doc
            .layer(layer)
            .filter(|l| !is_unplaced(l.paths()))
            .and_then(|l| l.paths().first())
            .map(|e| e.path.clone());
        match (&side, surface) {
            (Some(LayerPath::Surface(_)), Some(false)) => {
                self.refuse(
                    Source::Path,
                    lang.pick(
                        "このレイヤーにはモデルの上のパスがあります",
                        "This layer has a path on the model",
                    ),
                );
                None
            }
            (Some(LayerPath::Canvas(_)), Some(true)) => {
                self.refuse(
                    Source::Path,
                    lang.pick(
                        "このレイヤーにはキャンバスのパスがあります",
                        "This layer has a canvas path",
                    ),
                );
                None
            }
            _ => Some((layer, existing)),
        }
    }

    /// パスを描いてレイヤーへ入れる（1 回の Undo）。`layer` が None なら、選んでいるレイヤーの上に新しいパスレイヤーを足す。レイヤーの一覧に同じ ID の
    /// パスがあれば置き換え、無ければ一覧の上に加える。入れたレイヤーと、面に投影できなかった標本の数。
    fn path_write(
        &mut self,
        layer: Option<LayerId>,
        path: &LayerPath,
    ) -> Result<(LayerId, usize), String> {
        let lang = self.lang;
        let core = |e: CoreError| lang.core_error(&e);
        let options = self.path_options();
        if let Some(l) = layer {
            let mut entries = self
                .doc
                .layer(l)
                .map(|x| x.paths().to_vec())
                .unwrap_or_default();
            let unplaced = is_unplaced(&entries);
            match entries.iter_mut().find(|e| e.id() == path.id()) {
                Some(e) => e.path = path.clone(),
                // 点の無い 1 本だけの一覧に別のパスを足すときは、その 1 本を新しいパスに替える（空のパスを残さない。側が違っても付けられる）
                None if unplaced => entries[0].path = path.clone(),
                None => entries.push(LayerPathEntry::new(path.clone())),
            }
            let gaps = self.path_write_list(l, entries)?;
            return Ok((l, gaps));
        }
        let (channels, gaps) = match path {
            LayerPath::Canvas(c) => {
                let r =
                    render_canvas(c, &options).map_err(|e| crate::lang::path_error(lang, &e))?;
                (r.channels, 0)
            }
            LayerPath::Surface(s) => {
                let ctx = self.path_surface_ctx_for(s)?;
                let r = render_surface(s, &ctx.render, &options)
                    .map_err(|e| crate::lang::path_error(lang, &e))?;
                let gaps = r.gaps;
                (r.channels, gaps)
            }
        };
        let n = self.doc.layers().iter().filter(|l| l.has_paths()).count() + 1;
        let above = self
            .selected_layer
            .filter(|id| self.doc.layer(*id).is_some());
        let id = self
            .doc
            .add_path_layer(&layer_name(lang, n), path.clone(), channels, above)
            .map_err(core)?;
        Ok((id, gaps))
    }

    /// 3D のパスの文脈（パスが今のモデルで描かれていなければ、その理由）。
    fn path_surface_ctx_for(&self, s: &SurfacePath) -> Result<SurfaceCtx, String> {
        let ctx = self.path_surface_ctx()?;
        if s.model_fingerprint != *ctx.fingerprint {
            return Err(self
                .lang
                .pick(
                    "別のモデルで描かれたパスです",
                    "The path was drawn on another model",
                )
                .into());
        }
        Ok(ctx)
    }

    /// レイヤーのパスの一覧を描いて入れ替える（1 回の Undo）。空の一覧はパスを外して、そのチャンネルを空にする。面に投影できなかった
    /// 標本の数を返す。
    pub(crate) fn path_write_list(
        &mut self,
        layer: LayerId,
        entries: Vec<LayerPathEntry>,
    ) -> Result<usize, String> {
        let lang = self.lang;
        let core = |e: CoreError| lang.core_error(&e);
        // 点が 1 つも無い 1 本だけのレイヤーを、別の側（2D・3D）のパスにするときは、core が「パスは種類を変えない」と断るので、
        // 先にパスを外してから付け直す（1 回の Undo。外すのは点の無いパスなので、画素は変わらない）
        let swaps_side = self.doc.layer(layer).is_some_and(|l| {
            is_unplaced(l.paths())
                && entries
                    .first()
                    .is_some_and(|e| e.path.is_canvas() != l.paths()[0].path.is_canvas())
        });
        let surface = entries.iter().find_map(|e| match &e.path {
            LayerPath::Surface(s) => Some(s.clone()),
            LayerPath::Canvas(_) => None,
        });
        let Some(first) = surface else {
            if swaps_side {
                self.doc
                    .batch(|d| {
                        d.set_paths(layer, Vec::new(), Vec::new())?;
                        d.set_canvas_paths(layer, entries)
                    })
                    .map_err(core)?;
            } else {
                self.doc.set_canvas_paths(layer, entries).map_err(core)?;
            }
            return Ok(0);
        };
        let ctx = self.path_surface_ctx_for(&first)?;
        let rendered = render_list(&entries, Some(&ctx.render), &self.path_options())
            .map_err(|e| crate::lang::path_error(lang, &e))?;
        let gaps = rendered.gaps;
        if swaps_side {
            self.doc
                .batch(|d| {
                    d.set_paths(layer, Vec::new(), Vec::new())?;
                    d.set_paths(layer, entries, rendered.channels)
                })
                .map_err(core)?;
        } else {
            self.doc
                .set_paths(layer, entries, rendered.channels)
                .map_err(core)?;
        }
        Ok(gaps)
    }

    /// パスをレイヤーへ入れ、選ぶ点を決める。入れたら true。断られたら理由を知らせて何も変えない。
    fn path_commit(
        &mut self,
        layer: Option<LayerId>,
        path: LayerPath,
        select: Option<usize>,
    ) -> bool {
        let lang = self.lang;
        match self.path_write(layer, &path) {
            Ok((id, gaps)) => {
                if layer.is_none() {
                    self.selected_layer = Some(id);
                    self.set_edit_mask(false);
                }
                self.path.active = Some(ActivePath {
                    layer: id,
                    path: path.id(),
                });
                self.path.fresh = None;
                self.path.selected = select.map(|index| PointRef {
                    layer: id,
                    path: path.id(),
                    index,
                });
                self.modified = true;
                if gaps > 0 {
                    self.warn(Source::Path, gaps_message(lang, gaps));
                }
                true
            }
            Err(m) => {
                self.fail(Source::Path, m);
                false
            }
        }
    }

    /// ツールが替わった（途中のドラッグ・スライダーの値・名前の入力を捨てる。選んだ点も外す）。
    pub fn path_tool_changed(&mut self) {
        self.path.drag = None;
        self.path.pen_down = None;
        self.path.pending = None;
        self.path.selected = None;
        self.path.marked = None;
        self.path.rect = None;
        self.path.renaming = None;
        self.path.preset_renaming = None;
    }

    /// パスの編集を抜ける（Esc・Enter・「新しいパス」）: 選んだ点と一覧で選んだパスを外し、次に置く点は、選んでいるレイヤーの一覧に新しい
    /// パスを始める。抜けたか（編集しているパスが無ければ何もしない）。
    pub fn path_exit(&mut self) -> bool {
        let Some((layer, _)) = self.path_active_index() else {
            return false;
        };
        self.path.selected = None;
        self.path.active = None;
        // 点が 1 つも無い 1 本だけのレイヤー（新規パスレイヤー・点を全部消したレイヤー）は、抜けても次の点がその 1 本に入る
        // （空のパスを残して 2 本目を始めない。側（2D・3D）も、その点で決まる）
        let unplaced = self
            .doc
            .layer(layer)
            .is_some_and(|l| is_unplaced(l.paths()));
        self.path.fresh = (!unplaced).then_some(layer);
        true
    }

    /// Esc: ドラッグを捨てる（そのフレームは選んだ点を残す）。ドラッグが無ければ選んだ点を外し、点も選んでいなければパスの編集を
    /// 抜ける。何かあったか。`frame` は今のフレームの
    /// 番号（2D と 3D が両方見えていても、同じフレームの Esc は 1 回だけ扱う）。
    pub fn path_cancel(&mut self, frame: u64) -> bool {
        if !self.tool.is_path() {
            return false;
        }
        if self.path.esc_frame == Some(frame) {
            return true;
        }
        // 名前の入力欄が受けた Esc（欄が先に手放していても、1 つ前のフレームで入力中だったかで分かる）
        if self
            .path
            .typing_frame
            .is_some_and(|t| frame.saturating_sub(t) <= 1)
        {
            return false;
        }
        self.path.pending = None;
        let any = if self.path.drag.take().is_some() {
            self.path.pen_down = None;
            self.info(
                Source::Path,
                self.lang
                    .pick("点の移動をやめました。", "Point move cancelled."),
            );
            true
        } else if self.path.rect.take().is_some() {
            true
        } else if self.path.selected.is_some() || self.path.marked.is_some() {
            self.path.selected = None;
            self.path.marked = None;
            true
        } else {
            self.path_exit()
        };
        if any {
            self.path.esc_frame = Some(frame);
        }
        any
    }

    /// 点 `index` の、今の曲がりのままの取っ手（2D は画素、3D は休みの形のモデルの空間）。3D でモデルが無ければ理由。
    pub fn path_initial_handles(&self, index: usize) -> Result<([f64; 3], [f64; 3]), String> {
        let Some((_, path)) = self.path_layer() else {
            return Err(String::new());
        };
        let points: Vec<curve::P3> = match path {
            LayerPath::Canvas(c) => c.points.iter().map(|p| [p.x, p.y, 0.0]).collect(),
            LayerPath::Surface(s) => {
                let ctx = self.path_surface_ctx_for(s)?;
                s.points
                    .iter()
                    .map(|p| {
                        paths::point_position(&ctx.render, p)
                            .map_or([0.0; 3], |(v, _)| v.as_dvec3().to_array())
                    })
                    .collect()
            }
        };
        Ok(curve::initial_handles(&points, index))
    }

    /// 取っ手のドラッグを当てる新しい接線（`vector` は動かした側の新しい向き）。もう折れている取っ手と Alt はその側だけ、Ctrl は
    /// 両方を同じ割合で伸ばし、それ以外は反対の側を向きだけ合わせる（長さは今のまま）。
    pub fn path_handle_tangent(
        old: edit::TangentValue,
        side: HandleSide,
        vector: [f64; 3],
        input: PathInputState,
    ) -> edit::TangentValue {
        use yolu_core::glam::DVec3;
        let (incoming, outgoing) = match old {
            edit::TangentValue::Handles { incoming, outgoing } => {
                (DVec3::from_array(incoming), DVec3::from_array(outgoing))
            }
            _ => (DVec3::ZERO, DVec3::ZERO),
        };
        let v = DVec3::from_array(vector);
        let (moved_old, other) = match side {
            HandleSide::In => (incoming, outgoing),
            HandleSide::Out => (outgoing, incoming),
        };
        // 折れている: 反対の向きが -moved と 1° 以上ずれている
        let broken = moved_old.length() > 1e-12
            && other.length() > 1e-12
            && (-moved_old.normalize()).dot(other.normalize()) < 1.0f64.to_radians().cos();
        let other = if input.alt || broken || v.length() <= 1e-12 {
            other
        } else if input.ctrl && moved_old.length() > 1e-12 {
            -v.normalize() * other.length() * (v.length() / moved_old.length())
        } else {
            -v.normalize() * other.length()
        };
        let (incoming, outgoing) = match side {
            HandleSide::In => (v, other),
            HandleSide::Out => (other, v),
        };
        edit::TangentValue::Handles {
            incoming: incoming.to_array(),
            outgoing: outgoing.to_array(),
        }
    }

    /// ドラッグを終える（離した・フォーカスを失った・離したのを取りこぼした）。ほとんど動かしていなければクリック（点を選ぶだけ）、
    /// 動かしていれば、そこまでの置き場所へ 1 回で動かす。取っ手のドラッグは、その点の接線を 1 回で替える。
    pub fn path_finish_drag(&mut self) {
        let Some(d) = self.path.drag.take() else {
            return;
        };
        self.path.pen_down = None;
        if d.moved < CLICK_RADIUS {
            return;
        }
        if let Some(side) = d.handle {
            let same = self.selected_layer == Some(d.layer)
                && self
                    .path_layer()
                    .is_some_and(|(_, p)| p.id() == d.path && d.index < p.point_count());
            let old = self
                .path_layer()
                .and_then(|(_, p)| edit::point_tangent(p, d.index));
            if let (true, Some(old), Some(vector)) = (same, old, d.vector) {
                let tangent = Self::path_handle_tangent(old, side, vector, self.path.input);
                self.path_apply(PathAction::Point(PointOp::Tangent {
                    index: d.index,
                    tangent,
                }));
            }
            return;
        }
        let Some(place) = d.target else {
            return;
        };
        // 押している間にレイヤー・パスが替わっていたら当てない
        let same = self.selected_layer == Some(d.layer)
            && self
                .path_layer()
                .is_some_and(|(_, p)| p.id() == d.path && d.index < p.point_count());
        if same {
            self.path_apply(PathAction::Point(PointOp::Move {
                index: d.index,
                place,
            }));
        }
    }

    /// パスの操作を当てる。
    pub fn path_apply(&mut self, action: PathAction) {
        let lang = self.lang;
        match action {
            PathAction::Select(index) => {
                self.path.selected = match (index, self.path_layer()) {
                    (Some(i), Some((layer, p))) if i < p.point_count() => Some(PointRef {
                        layer,
                        path: p.id(),
                        index: i,
                    }),
                    _ => None,
                };
            }
            PathAction::Point(op) => self.path_point(op),
            PathAction::SelectPath(id) => self.path_select_path(id),
            PathAction::List(op) => self.path_list(op),
            PathAction::SetCorner(on) => {
                let tangent = if on {
                    edit::TangentValue::Corner
                } else {
                    edit::TangentValue::Smooth
                };
                let ops = self
                    .path_selected_indices()
                    .into_iter()
                    .map(|index| PointOp::Tangent { index, tangent })
                    .collect();
                self.path_points_many(ops);
            }
            PathAction::SetHandles(on) => {
                let mut ops = Vec::new();
                for index in self.path_selected_indices() {
                    let tangent = if on {
                        match self.path_initial_handles(index) {
                            Ok((incoming, outgoing)) => {
                                edit::TangentValue::Handles { incoming, outgoing }
                            }
                            Err(m) => {
                                self.refuse(Source::Path, m);
                                return;
                            }
                        }
                    } else {
                        edit::TangentValue::Smooth
                    };
                    ops.push(PointOp::Tangent { index, tangent });
                }
                self.path_points_many(ops);
            }
            PathAction::Symmetry(on) => self.path_set_symmetry(on),
            PathAction::Preset(op) => self.path_preset(op),
            PathAction::Widths(pressure) => {
                let ops = self
                    .path_selected_indices()
                    .into_iter()
                    .map(|index| PointOp::Width { index, pressure })
                    .collect();
                self.path_points_many(ops);
            }
            PathAction::Reverse => {
                let Some((layer, Some(path))) = self.path_target(None) else {
                    return;
                };
                let n = path.point_count();
                let keep = self.path_selected_index().map(|i| n - 1 - i);
                let next = match &path {
                    LayerPath::Canvas(c) => LayerPath::Canvas(c.reversed()),
                    LayerPath::Surface(s) => LayerPath::Surface(s.reversed()),
                };
                self.path.marked = None;
                self.path_commit(Some(layer), next, keep);
            }
            PathAction::SelectAllPoints => {
                if let Some((layer, path)) = self.path_layer() {
                    let a = ActivePath {
                        layer,
                        path: path.id(),
                    };
                    let all = (0..path.point_count()).collect();
                    self.path.marked = Some((a, all));
                }
            }
            PathAction::Kind(kind) => self.path_set_kind(kind),
            PathAction::Style(edit) => self.path_style_edit(edit),
            PathAction::BeginRename(id) => {
                if let Some(layer) = self.selected_layer {
                    self.path_select_path(Some(id));
                    self.path_begin_rename(layer, id);
                }
            }
            PathAction::DeleteSelected => {
                let many = self.path_selected_indices();
                if many.len() > 1 {
                    // 後ろの点から消す（番号がずれない）
                    let ops = many.into_iter().rev().map(PointOp::Remove).collect();
                    self.path_points_many(ops);
                    return;
                }
                let target = self.path_layer().map(|(_, p)| {
                    self.path_selected_index().or(match p {
                        LayerPath::Canvas(c) => edit::last_index(&c.points),
                        LayerPath::Surface(s) => edit::last_index(&s.points),
                    })
                });
                if let Some(Some(i)) = target {
                    self.path_point(PointOp::Remove(i));
                }
            }
            PathAction::Brush(e) => self.path_brush_edit(e),
            PathAction::UseBrush => {
                let Some((layer, Some(path))) = self.path_target(None) else {
                    return;
                };
                let ctx = match &path {
                    LayerPath::Surface(_) => match self.path_surface_ctx() {
                        Ok(c) => Some(c),
                        Err(m) => {
                            self.refuse(Source::Path, m);
                            return;
                        }
                    },
                    LayerPath::Canvas(_) => None,
                };
                // 今のブラシの筆先の画像・角度・向きも写す
                let mut style = path.style().clone();
                style.tip = self.m2.brush.tip.image.clone();
                style.angle = self.m2.brush.tip.angle;
                style.follow = self.m2.brush.tip.follow_direction;
                let next = with_style(
                    &with_material(
                        &with_brush(&path, self.path_new_brush(ctx.as_ref())),
                        self.path_new_material(),
                    ),
                    style,
                );
                let keep = self.path_selected_index();
                self.path_commit(Some(layer), next, keep);
            }
            PathAction::Redraw => {
                let Some((layer, Some(path))) = self.path_target(None) else {
                    return;
                };
                let keep = self.path_selected_index();
                self.path_commit(Some(layer), path, keep);
            }
            PathAction::Rasterize(id) => {
                if self.is_stroking() {
                    self.refuse(Source::Path, crate::lang::refusals::during_stroke(lang));
                    return;
                }
                if let Some(reason) = self.read_only_reason().map(str::to_owned) {
                    self.refuse(
                        Source::Path,
                        crate::lang::refusals::read_only_set(lang, &reason),
                    );
                    return;
                }
                if self.doc.layer(id).is_none_or(|l| l.path().is_none()) {
                    return;
                }
                match self.doc.rasterize(id) {
                    Ok(()) => {
                        self.path.selected = None;
                        self.modified = true;
                        self.info(
                            Source::Path,
                            lang.pick("ラスタライズしました。", "Rasterized."),
                        );
                    }
                    Err(e) => self.notify(
                        crate::notice::Kind::of_core(&e),
                        Source::Path,
                        lang.core_error(&e),
                    ),
                }
            }
        }
    }

    fn path_point(&mut self, op: PointOp) {
        let lang = self.lang;
        let surface = match op {
            PointOp::Add(p) | PointOp::Insert { place: p, .. } | PointOp::Move { place: p, .. } => {
                Some(matches!(p, Place::Surface { .. }))
            }
            _ => None,
        };
        let Some((layer, existing)) = self.path_target(surface) else {
            return;
        };
        let (path, select, target) = match existing {
            // 点が 1 つも無い 1 本だけのパスは、置いた側（2D・3D）がパスの側と違えば、その側の 1 点のパスに替える（同じ ID・同じ一覧の位置）
            Some(path)
                if path.point_count() == 0
                    && self
                        .doc
                        .layer(layer)
                        .is_some_and(|l| is_unplaced(l.paths()))
                    && matches!(op, PointOp::Add(place)
                        if matches!(place, Place::Canvas { .. }) != path.is_canvas()) =>
            {
                let PointOp::Add(place) = op else { return };
                match self.path_new(place, Some(path.channel())) {
                    Ok(p) => (p.with_id(path.id()), Some(0), Some(layer)),
                    Err(m) => {
                        self.refuse(Source::Path, m);
                        return;
                    }
                }
            }
            Some(path) => match edit::apply(&path, &op) {
                Ok((next, select)) => (next, select, Some(layer)),
                Err(r) => {
                    self.refuse(Source::Path, crate::lang::refusals::path_edit(lang, r));
                    return;
                }
            },
            None => match op {
                // 編集を抜けたパスレイヤーなら、その一覧に新しいパスを始める（パスの無いレイヤーなら、新しいパスレイヤー）
                PointOp::Add(place) => match self.path_new(
                    place,
                    self.doc
                        .layer(layer)
                        .and_then(|l| l.paths().first())
                        .map(|e| e.path.channel()),
                ) {
                    // パスレイヤー・塗りつぶしレイヤーなら、そのレイヤーの一覧に加える（塗りつぶしレイヤーのパスは塗りつぶしの上に重なる）
                    Ok(p) => (
                        p,
                        Some(0),
                        self.doc
                            .layer(layer)
                            .is_some_and(|l| {
                                l.has_paths() || l.kind() == yolu_core::LayerKind::Fill
                            })
                            .then_some(layer),
                    ),
                    Err(m) => {
                        self.refuse(Source::Path, m);
                        return;
                    }
                },
                _ => return,
            },
        };
        self.path_commit(target, path, select);
    }

    /// パスの種類を替える（1 回の Undo）。パスを選んでいなければ、次に作るパスの種類にする。リボンの画像は先に読んでおく
    /// （文書へ渡してから描く）。
    fn path_set_kind(&mut self, kind: PathKind) {
        if let PathKind::Ribbon(r) = kind {
            let resource = crate::fx::inputs::resource_id(r.image);
            if let Err(m) = self.use_shelf_image(&resource) {
                self.refuse(Source::Path, m);
                return;
            }
        }
        let Some(path) = self.path_layer().map(|(_, p)| p.clone()) else {
            self.path.next_style.kind = kind;
            return;
        };
        if path.style().kind == kind {
            return;
        }
        let Some((layer, _)) = self.path_target(None) else {
            return;
        };
        let keep = self.path_selected_index();
        let mut style = path.style().clone();
        style.kind = kind;
        self.path_commit(Some(layer), with_style(&path, style), keep);
    }

    /// パスの対称を入れる・切る（1 回の Undo）。入れるときは、そのパスのレイヤーから見えて効いている対称定規の値を写す（2D のパスは 2D の対称定規、
    /// 3D のパスは 3D の対称定規。効いている対称定規が無ければ断る）。3D のパスの対称は鏡の面 1 枚だけで、線対称 2 本の対称定規だけが写せる。
    /// 3D のパスは休みの形のモデルの空間、対称定規は 3D ビューの形（ポーズを付けた形）の空間なので、ポーズを付けているあいだは入れない。
    fn path_set_symmetry(&mut self, on: bool) {
        let Some((layer, path)) = self.path_layer().map(|(l, p)| (l, p.clone())) else {
            return;
        };
        let lang = self.lang;
        let symmetry = match (on, &path) {
            (false, _) => paths::PathSymmetry::None,
            (true, LayerPath::Canvas(_)) => {
                let active = self.rulers_active_for(crate::rulers::Place::Canvas, Some(layer));
                match active.canvas_symmetry {
                    Some(s) => paths::PathSymmetry::Canvas(s),
                    None => {
                        let reason = self.no_symmetry_reason(crate::rulers::Place::Canvas, layer);
                        self.refuse(Source::Path, reason);
                        return;
                    }
                }
            }
            (true, LayerPath::Surface(_)) => {
                let posed = self
                    .view3d
                    .pose
                    .session
                    .as_ref()
                    .is_some_and(|s| s.is_posed());
                if posed {
                    self.refuse(
                        Source::Path,
                        lang.pick(
                            "ポーズを付けているときは入れられません",
                            "Not available while the model is posed",
                        ),
                    );
                    return;
                }
                let active = self.rulers_active_for(crate::rulers::Place::View3d, Some(layer));
                match active.surface_symmetry {
                    None => {
                        let reason = self.no_symmetry_reason(crate::rulers::Place::View3d, layer);
                        self.refuse(Source::Path, reason);
                        return;
                    }
                    Some(setup) => match (setup.mirror, setup.radial) {
                        (Some(plane), None) => paths::PathSymmetry::Mirror {
                            point: plane.point,
                            normal: plane.normal,
                        },
                        _ => {
                            self.refuse(
                                Source::Path,
                                lang.pick(
                                    "3D のパスの対称は鏡の面 1 枚だけです",
                                    "A 3D path can mirror across one plane only",
                                ),
                            );
                            return;
                        }
                    },
                }
            }
        };
        if path.style().symmetry == symmetry {
            return;
        }
        let Some((layer, _)) = self.path_target(None) else {
            return;
        };
        let keep = self.path_selected_index();
        let mut style = path.style().clone();
        style.symmetry = symmetry;
        self.path_commit(Some(layer), with_style(&path, style), keep);
    }

    /// 筆先・角度・向き・深さを替える（1 回の Undo）。パスを選んでいなければ、次に作るパスの設定にする。
    fn path_style_edit(&mut self, edit: StyleEdit) {
        let apply = |style: &mut PathStyle| match edit.clone() {
            StyleEdit::Tip(tip) => style.tip = tip,
            StyleEdit::Angle(a) => style.angle = a.clamp(-360.0, 360.0),
            StyleEdit::Follow(on) => style.follow = on,
            StyleEdit::Depth(d) => style.depth = d.map(|d| d.clamp(0.05, 64.0)),
        };
        let Some(path) = self.path_layer().map(|(_, p)| p.clone()) else {
            apply(&mut self.path.next_style);
            return;
        };
        let mut style = path.style().clone();
        apply(&mut style);
        if style == *path.style() {
            return;
        }
        let Some((layer, _)) = self.path_target(None) else {
            return;
        };
        let keep = self.path_selected_index();
        self.path_commit(Some(layer), with_style(&path, style), keep);
    }

    /// パスのブラシの値を替える。パスが無ければ、次に作るパスが取る今のブラシの値を替える。
    fn path_brush_edit(&mut self, e: BrushEdit) {
        let Some(path) = self.path_layer().map(|(_, p)| p.clone()) else {
            let b = &mut self.brush;
            match e {
                BrushEdit::Diameter(d) => {
                    b.radius = (d as f32 / 2.0).clamp(0.5, crate::state::MAX_RADIUS)
                }
                BrushEdit::Hardness(v) => b.hardness = v.clamp(0.0, 1.0) as f32,
                BrushEdit::Spacing(v) => b.spacing = v.clamp(0.01, 1.0) as f32,
                BrushEdit::Opacity(v) => b.opacity = v.clamp(0.0, 1.0) as f32,
                BrushEdit::Flow(v) => b.flow = v.clamp(0.0, 1.0) as f32,
                BrushEdit::PressureSize(on) => b.pressure_size = on,
                BrushEdit::PressureOpacity(on) => b.pressure_opacity = on,
                BrushEdit::PressureFlow(on) => b.pressure_flow = on,
                BrushEdit::AntiAlias(level) => b.anti_alias = level,
            }
            return;
        };
        let Some((layer, _)) = self.path_target(None) else {
            return;
        };
        let mut brush = path_brush(&path);
        let b = &mut brush.0;
        match e {
            BrushEdit::Diameter(d) => {
                let radius = match &path {
                    LayerPath::Canvas(_) => d / 2.0,
                    LayerPath::Surface(_) => match self.path_surface_ctx() {
                        Ok(ctx) => {
                            let unit = yolu_core::geometry::world_radius(
                                &ctx.render,
                                1.0,
                                self.doc.width(),
                            );
                            d / 2.0 * unit as f64
                        }
                        Err(m) => {
                            self.refuse(Source::Path, m);
                            return;
                        }
                    },
                };
                b.radius = radius.clamp(
                    if path.is_canvas() { 0.5 } else { 1e-6 },
                    if path.is_canvas() { 4096.0 } else { 1e6 },
                );
            }
            BrushEdit::Hardness(v) => b.hardness = v.clamp(0.0, 1.0),
            BrushEdit::Spacing(v) => b.spacing = v.clamp(0.01, 4.0),
            BrushEdit::Opacity(v) => b.opacity = v.clamp(0.0, 1.0),
            BrushEdit::Flow(v) => b.flow = v.clamp(0.0, 1.0),
            BrushEdit::PressureSize(on) => b.pressure_size = on,
            BrushEdit::PressureOpacity(on) => b.pressure_opacity = on,
            BrushEdit::PressureFlow(on) => b.pressure_flow = on,
            BrushEdit::AntiAlias(level) => b.anti_alias = level,
        }
        if brush == path_brush(&path) {
            return;
        }
        let keep = self.path_selected_index();
        // 組を持たないパスの色はブラシの色なので、ブラシの値を替えても色は変えない
        self.path_commit(Some(layer), with_brush(&path, brush), keep);
    }

    /// 3D のモデルが差し替わった（`view3d` が前のモデルを渡す）: 前のモデルに結び付いたパスを、すべてのテクスチャセットの文書で
    /// 新しいモデルへ付け直す。描き直せないものは画素にし（画素・透明度のロックで描き直せないものも）、すべてのロックのものだけ
    /// 前のモデルに結び付けたまま残す。短い知らせ（件数と、最初の 1 件の理由）を出し、レイヤーごとの結果を返す。
    ///
    /// 3D の点を掴んでいる最中なら、そのドラッグは捨てる。置き場所の三角形の番号は前のモデルのもので、付け直したパスには当てられない
    /// （範囲外なら断られ、範囲内なら無関係の三角形へ動いてしまう）。
    pub fn path_rebind_after_model(&mut self, old: &Arc<ViewModel>) -> Vec<rebind::PathReport> {
        let Some(new) = self.view3d.full_model().cloned() else {
            return Vec::new();
        };
        if Arc::ptr_eq(old, &new) {
            return Vec::new();
        }
        let lang = self.lang;
        if self.path.drag.is_some_and(|d| d.surface) {
            self.path.drag = None;
            if self.path.pen_down.is_some_and(|p| p.surface) {
                self.path.pen_down = None;
            }
            self.warn(
                Source::Path,
                lang.pick(
                    "モデルが替わったので、点の移動をやめました。",
                    "The model changed, so the point move was cancelled.",
                ),
            );
        }
        let mut all = Vec::new();
        for i in 0..self.sets.len() {
            let Some(set) = self.sets.get(i) else {
                continue;
            };
            if set.read_only.is_some() {
                continue;
            }
            // 今のセットは 3D ビューが描くマテリアル（試しの立方体は 0）、ほかのセットは結び付いたマテリアル
            let material = if i == self.sets.current_index() {
                self.view3d.material
            } else {
                set.bound.map_or(-1, |m| m as i32)
            };
            let doc = self.set_doc_mut(i);
            if doc
                .layers()
                .iter()
                .all(|l| !matches!(l.path(), Some(LayerPath::Surface(_))))
            {
                continue;
            }
            // 位置は両方のモデルの休みの形で比べる（前のモデルにポーズが付いていても、その形へ置き直さない）
            all.extend(rebind::rebind_document(
                doc,
                old.rest_geometry(),
                new.rest_geometry(),
                material,
                lang,
            ));
        }
        if !all.is_empty() {
            self.modified = true;
            // 全部を描き直せたなら済んだ知らせ。画素にした・残したパスがあれば気をつけること
            let text = rebind_message(lang, &all);
            if all.iter().all(|r| r.outcome == rebind::Outcome::Redrawn) {
                self.info(Source::Path, text);
            } else {
                self.warn(Source::Path, text);
            }
        }
        all
    }
}

/// 面に投影できなかった標本の知らせ。
fn gaps_message(lang: Lang, gaps: usize) -> String {
    lang.pick(
        format!("{gaps} 個の標本が面から外れて、描いていません"),
        format!("{gaps} sample(s) were off the surface and skipped"),
    )
}

/// モデルの差し替えの知らせ: 結果ごとの件数（0 は出さない）と、最初の 1 件の理由（レイヤー名つき。画素にした・残したものを先に）。
fn rebind_message(lang: Lang, reports: &[rebind::PathReport]) -> String {
    let count = |outcome: rebind::Outcome| reports.iter().filter(|r| r.outcome == outcome).count();
    let mut parts: Vec<String> = Vec::new();
    for (outcome, ja, en) in [
        (rebind::Outcome::Redrawn, "描き直し", "redrawn"),
        (rebind::Outcome::Rasterized, "画素にし", "rasterized"),
        (rebind::Outcome::KeptLocked, "残し", "kept"),
    ] {
        let n = count(outcome);
        if n == 0 {
            continue;
        }
        parts.push(lang.pick(
            format!("{n} 本を{ja}"),
            if parts.is_empty() {
                format!("{n} path(s) {en}")
            } else {
                format!("{n} {en}")
            },
        ));
    }
    let mut text = lang.pick(
        format!("モデルを差し替えて、パス {}ました。", parts.join("、")),
        format!("After the model change, {}.", parts.join(", ")),
    );
    let first = reports
        .iter()
        .find(|r| r.outcome != rebind::Outcome::Redrawn && r.reason.is_some())
        .or_else(|| reports.iter().find(|r| r.reason.is_some()));
    if let Some((name, reason)) = first.and_then(|r| Some((&r.name, r.reason.as_deref()?))) {
        let name = lang.quote(name);
        text.push_str(lang.pick("", " "));
        text.push_str(&lang.with_reason(
            lang.pick(
                format!("レイヤー{name}のパスは描き直せません"),
                format!("Cannot redraw the path of layer {name}"),
            ),
            reason,
        ));
    }
    text
}
