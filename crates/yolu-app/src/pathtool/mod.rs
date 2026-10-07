//! パスの道具（P）: 2D のキャンバスと 3D のモデルの面の上に、編集できるパス（制御点を通る曲線）を引く。層はパスから描いた画素を持ち、
//! パスと画素はいつも一緒に変わる（点を足す・差し込む・動かす・消す・閉じる・開く・太さを変える・ブラシや組を変える、どれも描き直して
//! 1 回の Undo）。パスの層には手で描けず、ラスタライズでパスを外すと今の画素だけが残る。
//!
//! - 2D のパスは画素の座標に、3D のパスはモデルの三角形と重心座標に結び付く（指紋が違うモデルでは編集せず、`rebind` で付け直す）。
//!   1 つの層はどちらか一方だけを持つ。曲線は core が点を順に通る centripetal Catmull-Rom で描き、`curve` が同じ式で画面に見せる。
//! - 点の操作は `edit`（点の並びだけの純粋な関数）、入力と重ね表示は 2D が `canvas`、3D が `surface`。入力は
//!   ストロークと同じ道（押す・動く・離す・Esc・フォーカスを失う・取りこぼし）から呼び、点のドラッグは離したとき 1 回で当てる。
//! - 組（マテリアル）がオンなら、パスは組のチャンネルの全部を 1 回で描く。
//! - 3D の点の三角形の番号は、隠したマテリアルを除いた「見せる形」ではなく「受けたままの形」の番号（保存と Unity 版と同じ）。

pub mod canvas;
pub mod curve;
pub mod edit;
pub mod rebind;
pub mod surface;

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::sync::{Arc, Mutex};

use egui::{Color32, Pos2};
use yolu_core::paths::{
    self, render_canvas, render_surface, CanvasPath, ChannelPaint, Options, PathBrush, SurfacePath,
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

/// 選んでいる点（層とパスの ID で確かめる。層・パスが替わったら無効）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PointRef {
    pub layer: LayerId,
    pub path: u128,
    pub index: usize,
}

/// ポインタの下に何があるか。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Hover {
    None,
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
}

/// ペンが触れている間のペンの番号と、触れたビュー（2D のキャンバスと 3D のビューが並んで見えていても、離したのを受け取るのは
/// 触れたビューだけ。相手のビューが離したサンプルで `pen_down` を下ろすと、触れたビューのドラッグが取り残される）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PenDown {
    pub id: u32,
    /// 3D のビューが触れたか（false なら 2D のキャンバス）。
    pub surface: bool,
}

/// パスの道具の状態（パスそのものは文書が持つ）。
#[derive(Default)]
pub struct PathState {
    pub selected: Option<PointRef>,
    pub drag: Option<PointDrag>,
    /// ペンが触れている間のペンの番号と触れたビュー。
    pub pen_down: Option<PenDown>,
    /// スライダーを動かしている間の値（キー・値）。離したとき 1 回で当てる。
    pub pending: Option<(&'static str, f32)>,
    /// Esc を扱ったフレーム（2D と 3D の両方が見えていて、同じ Esc を 2 回扱わないため）。
    esc_frame: Option<u64>,
    /// 3D の指紋（ポインタ・世代で覚える。三角形の数だけかかるので毎回は求めない）。
    fingerprint: Mutex<Option<(usize, u32, Arc<str>)>>,
}

impl PathState {
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
}

/// パスの操作（メニュー・キー・ボタン・試験が同じ道を通る）。
#[derive(Clone, Debug, PartialEq)]
pub enum PathAction {
    /// 選んでいる点を替える（None で外す）。文書は変えない。
    Select(Option<usize>),
    /// 点の操作。選んでいる層にパスが無く、足す操作なら、その上に新しいパスの層を作る。
    Point(PointOp),
    /// 選んでいる点（無ければ最後の点）を消す。
    DeleteSelected,
    /// パスのブラシの値（パスが無いときは、次に作るパスが取る今のブラシ）。
    Brush(BrushEdit),
    /// 今のブラシ・マテリアルで塗るチャンネルの組をパスに使う。
    UseBrush,
    /// 今のモデル・ポーズで描き直す。
    Redraw,
    /// パスを外して今の画素だけを残す。
    Rasterize(LayerId),
}

impl PathAction {
    /// 文書を変える操作か（読むだけのセットでは断る）。
    pub fn edits_document(&self) -> bool {
        !matches!(self, PathAction::Select(_))
    }
}

/// 3D の文脈（受けたままのモデル・その指紋・描くテクスチャセットのマテリアル）。
pub struct SurfaceCtx {
    pub model: Arc<ViewModel>,
    pub fingerprint: Arc<str>,
    pub material: i32,
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

/// パスを持つ層の名前。
fn layer_name(lang: Lang, n: usize) -> String {
    format!("{} {n}", lang.pick("パス", "Path"))
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
    /// 選んでいる層とそのパス（無ければ None）。
    pub fn path_layer(&self) -> Option<(LayerId, &LayerPath)> {
        let id = self.selected_layer?;
        Some((id, self.doc.layer(id)?.path()?))
    }

    /// 選んでいる点の番号（層・パスが替わっていたり、点が無くなっていたら None）。
    pub fn path_selected_index(&self) -> Option<usize> {
        let r = self.path.selected?;
        let (layer, path) = self.path_layer()?;
        (r.layer == layer && r.path == path.id() && r.index < path.point_count()).then_some(r.index)
    }

    /// パスの層が描くときの予算（文書のもの）。
    fn path_options(&self) -> Options<'static> {
        Options {
            width: self.doc.width(),
            height: self.doc.height(),
            tile_size: self.doc.tile_size(),
            source_budget_bytes: self.doc.source_budget_bytes(),
            stroke_budget_bytes: self.doc.stroke_budget_bytes(),
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
                    &ctx.model.geometry,
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

    /// 点を 1 つ持つ新しいパス（今のブラシ・描くチャンネル・組で）。
    fn path_new(&self, place: Place) -> Result<LayerPath, String> {
        let (id, channel, material) =
            (random_id(), self.m2.paint_channel, self.path_new_material());
        let empty = match place {
            Place::Canvas { .. } => LayerPath::Canvas(CanvasPath {
                id,
                channel,
                brush: self.path_new_brush(None),
                points: Vec::new(),
                material,
            }),
            Place::Surface { .. } => {
                let ctx = self.path_surface_ctx()?;
                LayerPath::Surface(SurfacePath {
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

    /// 編集する前の確かめ: 描ける状態か、選んでいる層とそのパス。`surface` は編集するのが 3D のパスか（None なら、あるパスのまま）。
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
        let existing = self.doc.layer(layer).and_then(|l| l.path().cloned());
        match (&existing, surface) {
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

    /// パスを描いて層へ入れる（1 回の Undo）。`layer` が None なら、選んでいる層の上に新しいパスの層を足す。入れた層と、面に投影できなかった
    /// 標本の数。
    fn path_write(
        &mut self,
        layer: Option<LayerId>,
        path: &LayerPath,
    ) -> Result<(LayerId, usize), String> {
        let lang = self.lang;
        let core = |e: CoreError| lang.core_error(&e);
        let options = self.path_options();
        let surface_ctx = |s: &SurfacePath, app: &AppState| -> Result<SurfaceCtx, String> {
            let ctx = app.path_surface_ctx()?;
            if s.model_fingerprint != *ctx.fingerprint {
                return Err(lang
                    .pick(
                        "別のモデルで描かれたパスです",
                        "The path was drawn on another model",
                    )
                    .into());
            }
            Ok(ctx)
        };
        match (layer, path) {
            (Some(l), LayerPath::Canvas(c)) => {
                self.doc.set_canvas_path(l, c.clone()).map_err(core)?;
                Ok((l, 0))
            }
            (Some(l), LayerPath::Surface(s)) => {
                let ctx = surface_ctx(s, self)?;
                let rendered = render_surface(s, &ctx.model.geometry, &options)
                    .map_err(|e| crate::lang::path_error(lang, &e))?;
                let gaps = rendered.gaps;
                self.doc
                    .set_path(l, path.clone(), rendered.channels)
                    .map_err(core)?;
                Ok((l, gaps))
            }
            (None, p) => {
                let (channels, gaps) = match p {
                    LayerPath::Canvas(c) => {
                        let r = render_canvas(c, &options)
                            .map_err(|e| crate::lang::path_error(lang, &e))?;
                        (r.channels, 0)
                    }
                    LayerPath::Surface(s) => {
                        let ctx = surface_ctx(s, self)?;
                        let r = render_surface(s, &ctx.model.geometry, &options)
                            .map_err(|e| crate::lang::path_error(lang, &e))?;
                        let gaps = r.gaps;
                        (r.channels, gaps)
                    }
                };
                let n = self
                    .doc
                    .layers()
                    .iter()
                    .filter(|l| l.path().is_some())
                    .count()
                    + 1;
                let above = self
                    .selected_layer
                    .filter(|id| self.doc.layer(*id).is_some());
                let id = self
                    .doc
                    .add_path_layer(&layer_name(lang, n), p.clone(), channels, above)
                    .map_err(core)?;
                Ok((id, gaps))
            }
        }
    }

    /// パスを層へ入れ、選ぶ点を決める。入れたら true。断られたら理由を知らせて何も変えない。
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
                self.path.selected = select.map(|index| PointRef {
                    layer: id,
                    path: path.id(),
                    index,
                });
                self.modified = true;
                if gaps > 0 {
                    self.warn(
                        Source::Path,
                        lang.pick(
                            format!("{gaps} 個の標本が面から外れて、描いていません"),
                            format!("{gaps} sample(s) were off the surface and skipped"),
                        ),
                    );
                }
                true
            }
            Err(m) => {
                self.fail(Source::Path, m);
                false
            }
        }
    }

    /// 道具が替わった（途中のドラッグ・スライダーの値を捨てる。選んだ点も外す）。
    pub fn path_tool_changed(&mut self) {
        self.path.drag = None;
        self.path.pen_down = None;
        self.path.pending = None;
        self.path.selected = None;
    }

    /// Esc: ドラッグを捨てる（そのフレームは選んだ点を残す）。ドラッグが無ければ選んだ点を外す。何かあったか。`frame` は今のフレームの
    /// 番号（2D と 3D が両方見えていても、同じフレームの Esc は 1 回だけ扱う）。
    pub fn path_cancel(&mut self, frame: u64) -> bool {
        if !self.tool.is_path() {
            return false;
        }
        if self.path.esc_frame == Some(frame) {
            return true;
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
        } else {
            self.path.selected.take().is_some()
        };
        if any {
            self.path.esc_frame = Some(frame);
        }
        any
    }

    /// ドラッグを終える（離した・フォーカスを失った・離したのを取りこぼした）。ほとんど動かしていなければクリック（点を選ぶだけ）、
    /// 動かしていれば、そこまでの置き場所へ 1 回で動かす。
    pub fn path_finish_drag(&mut self) {
        let Some(d) = self.path.drag.take() else {
            return;
        };
        self.path.pen_down = None;
        if d.moved < CLICK_RADIUS {
            return;
        }
        let Some(place) = d.target else {
            return;
        };
        // 押している間に層・パスが替わっていたら当てない
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
            PathAction::DeleteSelected => {
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
                let next = with_material(
                    &with_brush(&path, self.path_new_brush(ctx.as_ref())),
                    self.path_new_material(),
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
            Some(path) => match edit::apply(&path, &op) {
                Ok((next, select)) => (next, select, Some(layer)),
                Err(r) => {
                    self.refuse(Source::Path, crate::lang::refusals::path_edit(lang, r));
                    return;
                }
            },
            None => match op {
                PointOp::Add(place) => match self.path_new(place) {
                    Ok(p) => (p, Some(0), None),
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
                                &ctx.model.geometry,
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
    /// 前のモデルに結び付けたまま残す。短い知らせ（件数と、最初の 1 件の理由）を出し、層ごとの結果を返す。
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
            all.extend(rebind::rebind_document(
                doc,
                &old.geometry,
                &new.geometry,
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
