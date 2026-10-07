//! 範囲の道具の動き: バケツ（範囲を 1 回で塗る）、ポリゴン塗りつぶし（押したまま通った範囲を足し、離して 1 回の Undo）、ID の色で選択、
//! ポインタの下の範囲の求め方（強調）。入力の道（2D キャンバスと 3D ビュー）は `Where` で分け、範囲の求め方は同じ。

use std::collections::HashSet;
use std::sync::Arc;

use egui::{Pos2, Rect};
use yolu_core::geometry::{pick, SurfaceGeometry};
use yolu_core::glam::{DVec2, Vec2};
use yolu_core::material_triangles::PixelTriangle;
use yolu_core::{CoreError, Document, LayerId, SelectionMask, TriangleFill};

use super::kind_name;
use crate::canvas::view::CanvasView;
use crate::lang::Lang;
use crate::notice::Source;
use crate::state::{AppState, StrokeSource, Tool};

/// ポインタを置いた画面（座標の変換が違う）。
#[derive(Clone, Copy)]
pub enum Where<'a> {
    /// 2D キャンバス（今の表示の写し）。
    Canvas(&'a CanvasView),
    /// 3D ビュー（中身の表示域）。
    Surface(Rect),
}

impl Where<'_> {
    pub fn is_surface(&self) -> bool {
        matches!(self, Where::Surface(_))
    }
}

/// ポインタの下にあるもの。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Under {
    /// 今のセットの三角形。
    Triangle(u32),
    Nothing,
    /// ほかのテクスチャセットの面（3D だけ。その名前）。
    OtherSet(String),
}

/// ポインタの下の範囲の強調。
pub struct Hover {
    pub on_surface: bool,
    /// 強調の元の三角形（ID の色の強調では最初の三角形）。
    pub triangle: u32,
    /// 範囲の同一性（同じ範囲なら引き直さない）。
    pub key: u64,
    /// 強調する三角形（昇順）。
    pub tris: Arc<Vec<u32>>,
    /// 範囲の UV の輪郭（UV の座標）。
    pub outline: Arc<Vec<[Vec2; 2]>>,
    pub geometry: Arc<SurfaceGeometry>,
    /// 消すときの色（桃色）で出すか。
    pub erase: bool,
    /// ID の色の強調なら、作ったときの条件（マップ・ジオメトリ・色・許し幅）。ほかの道具が作った強調は None
    /// （強調の持ち主の印は強調と一緒に入れ替わる）。
    pub id_key: Option<(usize, usize, u32, u8)>,
    /// 3D: 今のカメラで手前に見える三角形（`overlay` が求める）。
    pub visible: Option<(yolu_core::geometry::CameraView, Arc<Vec<u32>>)>,
}

/// ポリゴン塗りつぶしのドラッグ。
pub struct PolygonDrag {
    fill: TriangleFill,
    on_surface: bool,
    keys: HashSet<u64>,
    last: Pos2,
    erase: bool,
}

impl PolygonDrag {
    /// 足した範囲の数。
    pub fn regions(&self) -> usize {
        self.keys.len()
    }
    pub fn on_surface(&self) -> bool {
        self.on_surface
    }
}

fn local(rect: Rect, p: Pos2) -> Vec2 {
    Vec2::new(p.x - rect.left(), p.y - rect.top())
}

/// 範囲の三角形の UV を、キャンバスの画素座標の三角形に（UV (0, 0) がキャンバスの左下）。
pub fn pixel_triangles(
    doc: &Document,
    geometry: &SurfaceGeometry,
    region: &[u32],
) -> Vec<PixelTriangle> {
    let (w, h) = (doc.width() as f64, doc.height() as f64);
    region
        .iter()
        .map(|&i| {
            let t = &geometry.triangles()[i as usize];
            [t.uv_a, t.uv_b, t.uv_c].map(|p| DVec2::new(p.x as f64 * w, p.y as f64 * h))
        })
        .collect()
}

/// ポインタの下の三角形。
pub fn under(app: &mut AppState, w: Where, at: Pos2) -> Under {
    let Some((model, material)) = app.region_model() else {
        return Under::Nothing;
    };
    match w {
        Where::Surface(rect) => {
            let view = app.view3d.camera.view(rect.width(), rect.height());
            match pick(&model.geometry, &view, local(rect, at)) {
                Some(hit) if hit.material == material => Under::Triangle(hit.triangle),
                Some(hit) => Under::OtherSet(model.material_name(hit.material as usize, app.lang)),
                None => Under::Nothing,
            }
        }
        Where::Canvas(view) => {
            let (x, y) = view.to_canvas(at);
            let (cw, ch) = (app.doc.width() as f64, app.doc.height() as f64);
            if !(0.0..cw).contains(&x) || !(0.0..ch).contains(&y) {
                return Under::Nothing;
            }
            let Some(grid) = app.region_grid() else {
                return Under::Nothing;
            };
            match grid.find(Vec2::new((x / cw) as f32, (y / ch) as f32)) {
                Some(t) => Under::Triangle(t),
                None => Under::Nothing,
            }
        }
    }
}

/// 読むだけのテクスチャセットなら、その短い理由（文書を変える道具の入口で断る文。ステータスバーへ）。
pub(super) fn read_only_message(app: &AppState) -> Option<String> {
    app.read_only_reason()
        .map(|reason| crate::lang::refusals::read_only_set(app.lang, reason))
}

/// 塗る・消すの前に確かめること（描けないときは短い理由）。返すのは塗る層。
pub(crate) fn paint_gate(app: &AppState) -> Result<LayerId, String> {
    let lang = app.lang;
    if let Some(message) = read_only_message(app) {
        return Err(message);
    }
    let Some(layer) = app.selected_layer.filter(|id| app.doc.layer(*id).is_some()) else {
        return Err(lang
            .pick("塗るレイヤーがありません", "No layer to paint")
            .into());
    };
    if let Some(reason) = app.paint_blocker() {
        return Err(reason);
    }
    Ok(layer)
}

/// マスクの「塗る」は白（見せる）、「消す」は黒（隠す）。白・黒はマスクのサムネイルの色なので、反転したマスクでは書く値を逆にする。
pub(crate) fn mask_reveals(app: &AppState, layer: LayerId, erase: bool) -> bool {
    let inverted = app
        .doc
        .layer(layer)
        .and_then(|l| l.mask())
        .is_some_and(|m| m.inverted());
    erase == inverted
}

/// 範囲の画素を塗る（今の選択範囲の内側だけ。マスクの編集中はマスク、マテリアルがオンなら組の全部）。画素が変わったか。
fn fill_with(app: &mut AppState, layer: LayerId, mask: &SelectionMask) -> Result<bool, CoreError> {
    let opacity = app.brush.opacity as f64;
    let erase = app.region.erase;
    if app.m2.edit_mask {
        let reveal = mask_reveals(app, layer, erase);
        return app.doc.fill_mask(layer, opacity, Some(mask), reveal);
    }
    let channels = app.paint_channels();
    app.doc
        .fill_material(layer, &channels, opacity, Some(mask), erase)
}

fn needs_model(app: &mut AppState) {
    app.refuse(Source::Fill, app.region_missing_reason());
}

fn other_set(app: &mut AppState, name: &str) {
    app.refuse(Source::Fill, other_set_face(app.lang, name));
}

/// ポインタの下の面がほかのテクスチャセット（`name`）のものだという断り。
pub(crate) fn other_set_face(lang: Lang, name: &str) -> String {
    let name = lang.quote(name);
    lang.pick(
        format!("ほかのテクスチャセット{name}の面です。"),
        format!("This face belongs to another texture set, {name}."),
    )
}

// ───────── バケツ ─────────

/// バケツ: 押した所の範囲を（今の選択範囲の内側だけ）塗る。1 回の Undo。
pub fn bucket(app: &mut AppState, w: Where, at: Pos2) {
    let lang = app.lang;
    let layer = match paint_gate(app) {
        Ok(l) => l,
        Err(m) => {
            app.refuse(Source::Fill, m);
            return;
        }
    };
    let (mask, what) = if app.region.by_color {
        let Where::Canvas(view) = w else {
            app.refuse(
                Source::Fill,
                lang.pick(
                    "近い色は 2D のキャンバスでだけ使えます。",
                    "Similar colors work only on the 2D canvas.",
                ),
            );
            return;
        };
        let (x, y) = view.to_canvas(at);
        if x < 0.0 || y < 0.0 || x >= app.doc.width() as f64 || y >= app.doc.height() as f64 {
            return;
        }
        super::bucket::start(app, vec![(x, y)]);
        return;
    } else {
        if app.region_model().is_none() {
            return needs_model(app);
        }
        let triangle = match under(app, w, at) {
            Under::Triangle(t) => t,
            Under::OtherSet(name) => return other_set(app, &name),
            Under::Nothing => {
                app.refuse(
                    Source::Fill,
                    lang.pick(
                        "ポインタの下にこのテクスチャセットの三角形がありません",
                        "No triangle of this texture set under the pointer",
                    ),
                );
                return;
            }
        };
        let kind = app.region.kind;
        let (Some(index), Some((model, _))) = (app.region_index(), app.region_model()) else {
            return needs_model(app);
        };
        let triangles = pixel_triangles(&app.doc, &model.geometry, index.region(triangle, kind));
        (
            SelectionMask::from_triangles(&app.doc, &triangles),
            kind_name(lang, kind),
        )
    };
    let mask = mask.and_then(|mask| {
        let margin = app.region.color.margin;
        let budget = app.doc.source_budget_bytes();
        if margin > 0 {
            mask.grow(margin as u32, budget)
        } else if margin < 0 {
            mask.shrink(margin.unsigned_abs() as u32, false, budget)
        } else {
            Ok(mask)
        }
    });
    let mask = match mask {
        Ok(m) => m,
        Err(e) => {
            app.notify(
                crate::notice::Kind::of_core(&e),
                Source::Fill,
                lang.core_error(&e),
            );
            return;
        }
    };
    let erase = app.region.erase;
    match fill_with(app, layer, &mask) {
        Ok(true) => {
            app.modified = true;
            if !erase && !app.m2.edit_mask {
                app.color.remember();
            }
            app.info(
                Source::Fill,
                format!(
                    "{what}{}",
                    if erase {
                        lang.pick("を消しました。", " erased.")
                    } else {
                        lang.pick("を塗りました。", " filled.")
                    }
                ),
            );
        }
        Ok(false) => app.refuse(
            Source::Fill,
            lang.pick("そこには塗るものがありません。", "Nothing to fill there."),
        ),
        Err(e) => app.notify(
            crate::notice::Kind::of_core(&e),
            Source::Fill,
            lang.core_error(&e),
        ),
    }
}

// ───────── ポリゴン塗りつぶし ─────────

/// 前の位置から今の位置までの線の上（画面で 4 px おき、多くて 64 点）。速く動かしても間の三角形を飛ばしにくい。
fn samples(from: Pos2, to: Pos2) -> Vec<Pos2> {
    let steps = ((from.distance(to) / 4.0).ceil() as usize).clamp(1, 64);
    (1..=steps)
        .map(|i| from + (to - from) * (i as f32 / steps as f32))
        .collect()
}

/// ポリゴン塗りつぶしを始める。始めたら true（以後 `drag_to` で範囲を足し、`finish_drag` で確定する）。
pub fn begin_polygon(app: &mut AppState, w: Where, at: Pos2) -> bool {
    let lang = app.lang;
    if app.region.drag.is_some() {
        return false;
    }
    if app.region_model().is_none() {
        needs_model(app);
        return false;
    }
    let layer = match paint_gate(app) {
        Ok(l) => l,
        Err(m) => {
            app.refuse(Source::Fill, m);
            return false;
        }
    };
    if let Under::OtherSet(name) = under(app, w, at) {
        other_set(app, &name);
        return false;
    }
    let (opacity, erase, mask) = (app.brush.opacity as f64, app.region.erase, app.m2.edit_mask);
    let fill = if mask {
        let reveal = mask_reveals(app, layer, erase);
        app.doc.begin_mask_triangle_fill(layer, opacity, reveal)
    } else {
        let channels = app.paint_channels();
        app.doc
            .begin_material_triangle_fill(layer, &channels, opacity, erase)
    };
    let fill = match fill {
        Ok(f) => f,
        Err(e) => {
            app.notify(
                crate::notice::Kind::of_core(&e),
                Source::Fill,
                lang.core_error(&e),
            );
            return false;
        }
    };
    if !erase && !mask {
        app.color.remember();
    }
    app.region.drag = Some(PolygonDrag {
        fill,
        on_surface: w.is_surface(),
        keys: HashSet::new(),
        last: at,
        erase,
    });
    add_region_at(app, w, at);
    // 最初の範囲で予算を超えたときは、`add_region_at` が札を手放している（始まっていない）
    app.region.drag.is_some()
}

/// ポインタの下の範囲を足す（足した範囲はもう塗ってある）。
fn add_region_at(app: &mut AppState, w: Where, at: Pos2) {
    let Under::Triangle(t) = under(app, w, at) else {
        return;
    };
    let kind = app.region.kind;
    let (Some(index), Some((model, _))) = (app.region_index(), app.region_model()) else {
        return;
    };
    let Some(drag) = app.region.drag.as_mut() else {
        return;
    };
    if !drag.keys.insert(index.key(t, kind)) {
        return;
    }
    let triangles = pixel_triangles(&app.doc, &model.geometry, index.region(t, kind));
    if let Err(e) = drag.fill.add(&mut app.doc, &triangles) {
        // core は失敗した塗りを取り消してから返す（予算を超えたなど）。札を手放して知らせる
        app.region.drag = None;
        app.canvas.stroke = None;
        app.view3d.stroke_ended();
        app.notify(
            crate::notice::Kind::of_core(&e),
            Source::Fill,
            app.lang.core_error(&e),
        );
    }
}

/// ドラッグで動いた。始めた画面の中だけで効く（もう一方の画面の入力は無視する）。
pub fn drag_to(app: &mut AppState, w: Where, at: Pos2) {
    let Some(drag) = app.region.drag.as_ref() else {
        return;
    };
    if drag.on_surface != w.is_surface() {
        return;
    }
    let from = drag.last;
    for p in samples(from, at) {
        if app.region.drag.is_none() {
            return;
        }
        add_region_at(app, w, p);
    }
    if let Some(drag) = app.region.drag.as_mut() {
        drag.last = at;
    }
}

/// ドラッグを終える（cancel なら捨てる）。ドラッグが無ければ false。
pub fn finish_drag(app: &mut AppState, cancel: bool) -> bool {
    if super::bucket::finish(app, cancel) {
        return true;
    }
    let Some(drag) = app.region.drag.take() else {
        return false;
    };
    let lang = app.lang;
    let regions = drag.keys.len();
    let what = kind_name(lang, app.region.kind);
    if cancel {
        drag.fill.cancel(&mut app.doc);
        app.info(
            Source::Fill,
            lang.pick("ストロークを取り消しました。", "Stroke cancelled."),
        );
        return true;
    }
    match drag.fill.commit(&mut app.doc) {
        Ok(result) => {
            if result.changed {
                app.modified = true;
                let text = if drag.erase {
                    format!(
                        "{what} × {regions} {}",
                        lang.pick("を消しました。", "erased.")
                    )
                } else {
                    format!(
                        "{what} × {regions} {}",
                        lang.pick("を塗りました。", "filled.")
                    )
                };
                app.info(Source::Fill, text);
            } else if regions == 0 {
                app.refuse(
                    Source::Fill,
                    lang.pick(
                        "ポインタの下にこのテクスチャセットの三角形がありません。",
                        "No triangle of this texture set under the pointer.",
                    ),
                );
            } else {
                app.refuse(
                    Source::Fill,
                    lang.pick("そこには塗るものがありません。", "Nothing to fill there."),
                );
            }
        }
        Err(e) => app.notify(
            crate::notice::Kind::of_core(&e),
            Source::Fill,
            lang.core_error(&e),
        ),
    }
    true
}

// ───────── 入口（キャンバス・3D ビューの入力から） ─────────

/// 2D キャンバスの押下（ブラシ以外のツール）。ドラッグを始めたら true。
pub fn canvas_press(
    app: &mut AppState,
    view: &CanvasView,
    at: Pos2,
    _source: StrokeSource,
) -> bool {
    let w = Where::Canvas(view);
    match app.tool {
        Tool::Fill if app.region.by_color && app.region.color.leftovers => {
            super::bucket::begin(app, view, at)
        }
        Tool::Fill => {
            bucket(app, w, at);
            false
        }
        Tool::IdSelect => {
            super::idcolor::select_by_id(app, w, at);
            false
        }
        Tool::PolygonFill => begin_polygon(app, w, at),
        _ => false,
    }
}

/// 3D ビューの押下（ブラシ以外のツール）。ドラッグを始めたら true。
pub fn surface_press(app: &mut AppState, rect: Rect, at: Pos2, _source: StrokeSource) -> bool {
    let w = Where::Surface(rect);
    match app.tool {
        Tool::Fill => {
            bucket(app, w, at);
            false
        }
        Tool::IdSelect => {
            super::idcolor::select_by_id(app, w, at);
            false
        }
        Tool::PolygonFill => begin_polygon(app, w, at),
        _ => false,
    }
}

// ───────── ポインタの下の範囲（強調） ─────────

/// この画面（`w`）のポインタの下の範囲を求め直す（範囲が変わったときだけ引き直す）。`at` が None（ポインタがこの画面に無い）なら、
/// この画面が作った強調だけを消す。2D と 3D を並べて出していると、ポインタの無い側が毎フレーム呼ぶので、ポインタのある側の強調を
/// 消してしまうと、その側は毎フレーム範囲と輪郭を作り直すことになる。道具が範囲を出さないときは、どの画面のものも消す。
pub fn update_hover(app: &mut AppState, w: Where, at: Option<Pos2>) {
    let tool = app.tool;
    let active = tool.is_region() && !(tool == Tool::Fill && app.region.by_color);
    let Some(at) = at.filter(|_| active) else {
        let mine = app
            .region
            .hover
            .as_ref()
            .is_some_and(|h| h.on_surface == w.is_surface());
        if !active || mine {
            app.region.hover = None;
        }
        return;
    };
    let Some((model, _)) = app.region_model() else {
        app.region.hover = None;
        return;
    };
    let erase = app.region.erase && tool != Tool::IdSelect;
    if tool == Tool::IdSelect {
        super::idcolor::update_hover(app, w, at);
        return;
    }
    let Under::Triangle(t) = under(app, w, at) else {
        app.region.hover = None;
        return;
    };
    let kind = app.region.kind;
    let Some(index) = app.region_index() else {
        app.region.hover = None;
        return;
    };
    let key = index.key(t, kind);
    if let Some(h) = app.region.hover.as_mut() {
        if h.id_key.is_none()
            && h.key == key
            && h.on_surface == w.is_surface()
            && Arc::ptr_eq(&h.geometry, &model.geometry)
        {
            h.triangle = t;
            h.erase = erase;
            return;
        }
    }
    let tris = index.region(t, kind).to_vec();
    let outline = index.outline(&tris);
    app.region.hover = Some(Hover {
        on_surface: w.is_surface(),
        triangle: t,
        key,
        tris: Arc::new(tris),
        outline: Arc::new(outline),
        geometry: model.geometry.clone(),
        erase,
        id_key: None,
        visible: None,
    });
}

/// 範囲の道具の強調と、ID の色の強調を試験で読む口。
impl AppState {
    pub fn region_hover_len(&self) -> Option<usize> {
        self.region.hover.as_ref().map(|h| h.tris.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn samples_step_every_four_points_and_end_at_the_target() {
        let s = samples(Pos2::new(0.0, 0.0), Pos2::new(40.0, 0.0));
        assert_eq!(s.len(), 10);
        assert_eq!(*s.last().unwrap(), Pos2::new(40.0, 0.0));
        assert_eq!(
            samples(Pos2::ZERO, Pos2::ZERO),
            vec![Pos2::ZERO],
            "動かなくても 1 点"
        );
        assert_eq!(
            samples(Pos2::ZERO, Pos2::new(10_000.0, 0.0)).len(),
            64,
            "多くて 64 点"
        );
    }
}
