//! 塗りつぶしの点のグラデーションの点を、3D ビューと 2D のキャンバスで置く・動かす・選ぶ・消す（`yolu_core::fill_points`）。
//!
//! - 編集に入っている間（欄の「点を編集」・点のグラデーションを追加した直後）だけ、左ボタン（ペンの接触）を受け取る。点の印の上なら選んで
//!   ドラッグ、モデルの面（2D ならその画素）の上なら点を追加して選び、そのままドラッグできる。押してから離すまでの変更は 1 回の Undo
//!   （`coalesce`）。Esc・ウィンドウのフォーカスの喪失では押す前に戻して履歴にも残さない。モデルの外・ほかのテクスチャセットの面は選びを外すだけ。
//! - モデルの空間の点は面の上の位置（3D ビューは当てた面の位置、2D は位置のマップのその画素）、UV の空間の点はその UV。
//! - 印は 3D ビューと 2D のキャンバスの両方に描く: モデルの空間の点の 2D の位置は面の上のいちばん近い点の UV、UV の空間の点の 3D の位置は
//!   位置のマップのその画素（無ければ 3D には描かない）。
//! - 選んだ点は Delete・Backspace で消す（最後の 1 つは消さない。点のグラデーションごと外すのは欄のボタン）。

use egui::{pos2, Color32, Pos2, Rect, Stroke, Ui};
use yolu_core::fill_points::{GradientPoint, PointGradient, PointSpace, MAX_POINTS};
use yolu_core::glam::{Vec2, Vec3};
use yolu_core::{Channel, ChannelKind, LayerId, LayerKind, Rgba8};

use super::gizmo::Source;
use crate::canvas::view::CanvasView;
use crate::notice::Source as NoticeSource;
use crate::state::AppState;

/// 印の半径と、掴める近さ（画面の点）。
pub const MARK_RADIUS: f32 = 6.0;
pub const GRAB_POINTS: f32 = 10.0;

/// 点のドラッグの途中。
#[derive(Clone, Debug, PartialEq)]
pub struct PointDrag {
    pub layer: LayerId,
    pub channel: Channel,
    pub index: usize,
    pub source: Source,
    /// ドラッグを始めた文書（テクスチャセットを替えたら、別の文書へ当てない）。
    pub doc_id: u128,
    /// 3D ビューか 2D のキャンバスか。
    pub in_3d: bool,
    /// 押したときに点を追加したか（動かさずに離しても、追加は残す）。
    pub added: bool,
}

/// 点を編集しているレイヤーとチャンネル（選んでいるレイヤーで、そのチャンネルに点のグラデーションがあるときだけ）。
pub fn target(app: &AppState) -> Option<(LayerId, Channel)> {
    // 編集のモードの点は、点の印で選んで G で動かす（`objects`）。ここの置く・掴むはペイントのモードだけ
    if !app.mode.paints() {
        return None;
    }
    let (layer, channel) = app.fillfx.edit_points?;
    if app.selected_layer != Some(layer) || app.m2.edit_mask {
        return None;
    }
    let l = app.doc.layer(layer)?;
    (l.kind() == LayerKind::Fill && l.fill_points(channel).is_some()).then_some((layer, channel))
}

fn gradient(app: &AppState, layer: LayerId, channel: Channel) -> Option<PointGradient> {
    app.doc.layer(layer)?.fill_points(channel).cloned()
}

/// 点の新しい色: メインの色（スカラーのチャンネルは、その明るさの灰色）。
fn main_color(app: &AppState, channel: Channel) -> Rgba8 {
    color_of(app, channel, app.color.main)
}

fn color_of(app: &AppState, channel: Channel, c: [f32; 4]) -> Rgba8 {
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    let scalar = app
        .doc
        .channel_info(channel)
        .is_some_and(|i| i.kind == ChannelKind::Scalar);
    if scalar {
        let y = byte(0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]);
        Rgba8::new(y, y, y, 255)
    } else {
        Rgba8::new(byte(c[0]), byte(c[1]), byte(c[2]), 255)
    }
}

/// 新しい点のグラデーション: モデルがあればモデルの空間で、モデルの外形の左右に 2 点（メインの色とサブの色）。無ければ UV の空間で左右に 2 点。
pub fn new_gradient(app: &AppState, channel: Channel) -> PointGradient {
    let (a, b) = (
        color_of(app, channel, app.color.main),
        color_of(app, channel, app.color.sub),
    );
    let point = |position, color| GradientPoint { position, color };
    match app.region_model() {
        Some((model, _)) => {
            let bounds = model.geometry.bounds();
            let (min, max) = (bounds.min(), bounds.max());
            let mid = (min + max) * 0.5;
            let at = |x: f32| [x as f64, mid.y as f64, mid.z as f64];
            PointGradient {
                space: PointSpace::Model,
                spread: yolu_core::fill_points::DEFAULT_SPREAD,
                points: vec![point(at(min.x), a), point(at(max.x), b)],
            }
        }
        None => PointGradient {
            space: PointSpace::Uv,
            spread: yolu_core::fill_points::DEFAULT_SPREAD,
            points: vec![point([0.25, 0.5, 0.0], a), point([0.75, 0.5, 0.0], b)],
        },
    }
}

/// 文書へ入れる（断られたら理由を出して false）。
fn put(
    app: &mut AppState,
    layer: LayerId,
    channel: Channel,
    g: PointGradient,
    coalesce: bool,
) -> bool {
    let revision = app.doc.revision();
    match app.doc.set_fill_points(layer, channel, Some(g), coalesce) {
        Ok(()) => {
            if app.doc.revision() != revision {
                app.modified = true;
            }
            true
        }
        Err(e) => {
            app.notify(
                crate::notice::Kind::of_core(&e),
                NoticeSource::FillLayer,
                app.lang.core_error(&e),
            );
            false
        }
    }
}

// ───────── 3D ビュー ─────────

/// 3D ビューの点（表示域の左上から）の下の、今のテクスチャセットの面の、その空間での位置。面が無い・ほかのセットの面なら None。
fn surface_at(app: &AppState, rect: Rect, at: Pos2, space: PointSpace) -> Option<[f64; 3]> {
    let (model, material) = app.region_model()?;
    let view = app.view3d.camera.view(rect.width(), rect.height());
    let gui = Vec2::new(at.x - rect.left(), at.y - rect.top());
    let hit = yolu_core::geometry::pick(&model.geometry, &view, gui)?;
    if hit.material != material {
        return None;
    }
    Some(match space {
        PointSpace::Model => [
            hit.position.x as f64,
            hit.position.y as f64,
            hit.position.z as f64,
        ],
        PointSpace::Uv => [hit.uv.x as f64, hit.uv.y as f64, 0.0],
    })
}

/// 点の 3D の位置（モデルの空間はそのまま、UV の空間は位置のマップのその画素）。
fn world_of(app: &AppState, g: &PointGradient, p: &GradientPoint) -> Option<Vec3> {
    let v = match g.space {
        PointSpace::Model => p.position,
        PointSpace::Uv => {
            let (w, h) = (app.doc.width(), app.doc.height());
            let x = (p.position[0] * w as f64).floor();
            let y = (p.position[1] * h as f64).floor();
            if x < 0.0 || y < 0.0 || x >= w as f64 || y >= h as f64 {
                return None;
            }
            app.doc.model_position_at(x as u32, y as u32)?
        }
    };
    Some(Vec3::new(v[0] as f32, v[1] as f32, v[2] as f32))
}

/// 点の印の画面の位置（3D ビュー。見えない点は None）。
fn screen_points_3d(app: &AppState, rect: Rect, g: &PointGradient) -> Vec<Option<Pos2>> {
    let view = app.view3d.camera.view(rect.width(), rect.height());
    g.points
        .iter()
        .map(|p| {
            world_of(app, g, p)
                .and_then(|w| view.to_screen(w))
                .map(|s| pos2(rect.left() + s.x, rect.top() + s.y))
        })
        .collect()
}

fn nearest(points: &[Option<Pos2>], at: Pos2) -> Option<usize> {
    points
        .iter()
        .enumerate()
        .filter_map(|(i, p)| p.map(|p| (i, p.distance(at))))
        .filter(|(_, d)| *d <= GRAB_POINTS)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i)
}

/// 左ボタン（ペンの接触）を押した（3D ビュー）。点の編集に入っていれば受け取って true（下のツールへ渡さない）。
pub fn press(app: &mut AppState, rect: Rect, at: Pos2, source: Source) -> bool {
    let Some((layer, channel)) = target(app) else {
        return false;
    };
    if app.is_stroking() || app.fillfx.point_drag.is_some() || app.fillfx.drag.is_some() {
        return false;
    }
    let Some(g) = gradient(app, layer, channel) else {
        return false;
    };
    let marks = screen_points_3d(app, rect, &g);
    let surface = surface_at(app, rect, at, g.space);
    begin(
        app,
        layer,
        channel,
        g,
        nearest(&marks, at),
        surface,
        source,
        true,
    )
}

/// 押した点を掴む・面の上に点を追加する（3D と 2D で同じ）。
#[allow(clippy::too_many_arguments)]
fn begin(
    app: &mut AppState,
    layer: LayerId,
    channel: Channel,
    mut g: PointGradient,
    grabbed: Option<usize>,
    surface: Option<[f64; 3]>,
    source: Source,
    in_3d: bool,
) -> bool {
    if let Some(reason) = app.read_only_reason() {
        app.refuse(
            NoticeSource::FillLayer,
            crate::lang::refusals::read_only_set(app.lang, reason),
        );
        return true;
    }
    app.doc.end_coalescing(); // 前の欄のドラッグにまとめない
    let (index, added) = match (grabbed, surface) {
        (Some(i), _) => (i, false),
        (None, Some(position)) => {
            if g.points.len() >= MAX_POINTS {
                app.refuse(
                    NoticeSource::FillLayer,
                    app.lang.with_reason(
                        app.lang.pick("点を追加できません", "Cannot add a point"),
                        app.lang.pick("点は 64 まで", "up to 64 points"),
                    ),
                );
                return true;
            }
            g.points.push(GradientPoint {
                position,
                color: main_color(app, channel),
            });
            if !put(app, layer, channel, g.clone(), true) {
                return true;
            }
            (g.points.len() - 1, true)
        }
        (None, None) => {
            // モデルの外・ほかのセットの面: 選びを外すだけ
            app.fillfx.point_selected = None;
            return true;
        }
    };
    app.fillfx.point_selected = Some(index);
    app.fillfx.point_drag = Some(PointDrag {
        layer,
        channel,
        index,
        source,
        doc_id: app.doc.id(),
        in_3d,
        added,
    });
    true
}

/// 掴んだ点を新しい位置へ（`position` はその空間の座標。None なら動かさない）。
fn move_to(app: &mut AppState, position: Option<[f64; 3]>) {
    let Some(d) = app.fillfx.point_drag.clone() else {
        return;
    };
    if app.doc.id() != d.doc_id {
        app.fillfx.point_drag = None;
        return;
    }
    if app.selected_layer != Some(d.layer) {
        release(app, true);
        return;
    }
    let (Some(position), Some(mut g)) = (position, gradient(app, d.layer, d.channel)) else {
        return;
    };
    let Some(p) = g.points.get_mut(d.index) else {
        return;
    };
    if p.position == position {
        return;
    }
    p.position = position;
    if !put(app, d.layer, d.channel, g, true) {
        release(app, false);
    }
}

/// ドラッグの途中（3D ビュー）。面の外へ出たら、最後に面の上だった所に置いたまま。
pub fn drag_to(app: &mut AppState, rect: Rect, at: Pos2) {
    let Some(d) = app.fillfx.point_drag.clone() else {
        return;
    };
    if !d.in_3d {
        return;
    }
    let space = gradient(app, d.layer, d.channel).map(|g| g.space);
    let position = space.and_then(|s| surface_at(app, rect, at, s));
    move_to(app, position);
}

/// ドラッグを終える。`commit` なら 1 回の Undo にまとめて確定、そうでなければ（Esc・フォーカスの喪失）押す前に戻して履歴にも残さない。
pub fn release(app: &mut AppState, commit: bool) {
    let Some(d) = app.fillfx.point_drag.take() else {
        return;
    };
    if d.doc_id != app.doc.id() {
        return;
    }
    if commit {
        app.doc.end_coalescing();
        return;
    }
    match app.doc.cancel_coalescing() {
        Ok(_) => {
            if d.added {
                app.fillfx.point_selected = None;
            }
        }
        Err(e) => {
            app.notify(
                crate::notice::Kind::of_core(&e),
                NoticeSource::FillLayer,
                app.lang.core_error(&e),
            );
            app.doc.end_coalescing();
        }
    }
}

/// ドラッグの途中か（どちらのビューでも）。
pub fn dragging(app: &AppState) -> bool {
    app.fillfx.point_drag.is_some()
}

/// 選んだ点を消す（最後の 1 つは断る）。1 回の Undo。
pub fn delete_selected(app: &mut AppState) {
    let Some((layer, channel)) = target(app) else {
        return;
    };
    let Some(i) = app.fillfx.point_selected else {
        return;
    };
    if gradient(app, layer, channel).is_some_and(|g| i >= g.points.len()) {
        app.fillfx.point_selected = None;
        return;
    }
    delete_point(app, layer, channel, i);
}

/// 点のグラデーションの `index` の点を消す（最後の 1 つは理由を出して断る）。1 回の Undo。消したら点の選びを外して true
/// （ペイントのモードの点の選びと、編集のモードの選んだ点の両方がここを通る）。
pub fn delete_point(app: &mut AppState, layer: LayerId, channel: Channel, index: usize) -> bool {
    let Some(mut g) = gradient(app, layer, channel) else {
        return false;
    };
    if index >= g.points.len() {
        return false;
    }
    if g.points.len() == 1 {
        app.refuse(
            NoticeSource::FillLayer,
            app.lang.with_reason(
                app.lang
                    .pick("点を削除できません", "Cannot delete the point"),
                app.lang.pick("最後の点", "it is the last point"),
            ),
        );
        return false;
    }
    g.points.remove(index);
    app.doc.end_coalescing();
    if !put(app, layer, channel, g, false) {
        return false;
    }
    if app.fillfx.edit_points == Some((layer, channel)) {
        app.fillfx.point_selected = None;
    }
    crate::objects::point_removed(app, layer, channel, index);
    true
}

fn mark_color(c: Rgba8) -> Color32 {
    Color32::from_rgb(c.r, c.g, c.b)
}

/// 印を描く（点の色の丸と白い縁。選んだ点は太い縁、ポインタの下の点は明るい縁）。
fn paint_marks(
    painter: &egui::Painter,
    marks: &[Option<Pos2>],
    g: &PointGradient,
    selected: Option<usize>,
    hover: Option<usize>,
) {
    for (i, (m, p)) in marks.iter().zip(&g.points).enumerate() {
        let Some(at) = m else {
            continue;
        };
        let chosen = selected == Some(i);
        let r = if chosen {
            MARK_RADIUS + 1.5
        } else {
            MARK_RADIUS
        };
        painter.circle_filled(*at, r + 2.0, Color32::from_black_alpha(160));
        painter.circle_filled(*at, r, mark_color(p.color));
        let edge = if chosen {
            Stroke::new(2.5, crate::view3d::shape_gizmo::HOVER)
        } else if hover == Some(i) {
            Stroke::new(2.0, Color32::from_gray(235))
        } else {
            Stroke::new(1.5, Color32::WHITE)
        };
        painter.circle_stroke(*at, r, edge);
    }
}

/// 3D ビューに印を重ねる。
pub fn draw(ui: &Ui, app: &mut AppState, rect: Rect, pointer: Option<Pos2>) {
    let Some((layer, channel)) = target(app) else {
        // 編集が終わった（レイヤーを替えた・隠した）ドラッグは取り残さない
        release(app, true);
        return;
    };
    let Some(g) = gradient(app, layer, channel) else {
        return;
    };
    let marks = screen_points_3d(app, rect, &g);
    let hover = pointer.and_then(|p| nearest(&marks, p));
    let painter = ui.painter_at(rect);
    paint_marks(&painter, &marks, &g, app.fillfx.point_selected, hover);
}

// ───────── 2D のキャンバス ─────────

/// キャンバスの画素の座標（左下が原点）の、その空間での位置（モデルの空間は位置のマップのその画素。覆っていなければ None）。
fn canvas_position(app: &AppState, x: f64, y: f64, space: PointSpace) -> Option<[f64; 3]> {
    let (w, h) = (app.doc.width(), app.doc.height());
    if x < 0.0 || y < 0.0 || x >= w as f64 || y >= h as f64 {
        return None;
    }
    match space {
        PointSpace::Uv => Some([x / w as f64, y / h as f64, 0.0]),
        PointSpace::Model => app.doc.model_position_at(x as u32, y as u32),
    }
}

/// 点の印のキャンバスの画面の位置（モデルの空間の点は、面の上のいちばん近い点の UV。モデルが無ければ None）。
fn screen_points_2d(app: &AppState, view: &CanvasView, g: &PointGradient) -> Vec<Option<Pos2>> {
    let (w, h) = (app.doc.width() as f64, app.doc.height() as f64);
    let model = app.region_model();
    g.points
        .iter()
        .map(|p| {
            let uv = match g.space {
                PointSpace::Uv => [p.position[0], p.position[1]],
                PointSpace::Model => {
                    let (model, material) = model.as_ref()?;
                    let at = Vec3::new(
                        p.position[0] as f32,
                        p.position[1] as f32,
                        p.position[2] as f32,
                    );
                    let hit = model
                        .geometry
                        .find_closest_point(at, f32::INFINITY, Vec3::ZERO, 1 << 20, *material)
                        .ok()??;
                    [hit.uv.x as f64, hit.uv.y as f64]
                }
            };
            Some(view.to_screen(uv[0] * w, uv[1] * h))
        })
        .collect()
}

/// 左ボタン（ペンの接触）を押した（2D のキャンバス）。点の編集に入っていれば受け取って true。
pub fn canvas_press(app: &mut AppState, view: &CanvasView, p: Pos2, source: Source) -> bool {
    let Some((layer, channel)) = target(app) else {
        return false;
    };
    if app.fillfx.point_drag.is_some() {
        return true;
    }
    let Some(g) = gradient(app, layer, channel) else {
        return false;
    };
    let marks = screen_points_2d(app, view, &g);
    let (x, y) = view.to_canvas(p);
    let surface = canvas_position(app, x, y, g.space);
    begin(
        app,
        layer,
        channel,
        g,
        nearest(&marks, p),
        surface,
        source,
        false,
    )
}

/// ドラッグの途中（2D のキャンバス）。
pub fn canvas_drag(app: &mut AppState, view: &CanvasView, p: Pos2) {
    let Some(d) = app.fillfx.point_drag.clone() else {
        return;
    };
    if d.in_3d {
        return;
    }
    let (x, y) = view.to_canvas(p);
    let position =
        gradient(app, d.layer, d.channel).and_then(|g| canvas_position(app, x, y, g.space));
    move_to(app, position);
}

/// キャンバスの印を描く。
pub fn paint_canvas(
    painter: &egui::Painter,
    view: &CanvasView,
    app: &AppState,
    pointer: Option<Pos2>,
) {
    let Some((layer, channel)) = target(app) else {
        return;
    };
    let Some(g) = gradient(app, layer, channel) else {
        return;
    };
    let marks = screen_points_2d(app, view, &g);
    let hover = pointer.and_then(|p| nearest(&marks, p));
    paint_marks(painter, &marks, &g, app.fillfx.point_selected, hover);
}
