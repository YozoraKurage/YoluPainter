//! 3D ビューの形のギズモの操作と描画（Unity 版の `TexturePaintWindow.ShapeGizmo`）: 選んでいる塗りつぶしレイヤーの、投影の置き場（型の上の
//! 投影とデカールの箱）か、3D ビューで編集している塗りつぶしのグラデーションの形を、モデルの面の上で動かす・回す・大きさを変える。
//!
//! - 出る条件: 3D のモデルがあり、選んでいるレイヤーが塗りつぶしで、マスクを編集していない。グラデーションを「3D ビューで編集」にしていればその形
//!   （ギズモは 1 つなのでグラデーションが先）、そうでなければ投影が UV 以外のとき置き場を出す（Q で隠す。Substance の Show/Hide manipulator）。
//!   フィルターの欄で形のグラデーション・画像の Generator のハンドルを出していれば、レイヤーの種類とマスクに依らず、それが一番先。
//! - ハンドルを押したときだけ受け取り（ハンドルの無い所の押下は今のツールへ）、離すまでの変更は 1 回の Undo にまとめる（`coalesce`）。
//!   Esc・ウィンドウのフォーカスの喪失・描き始めでは、ドラッグの前に戻して履歴にも残さない（`cancel_coalescing`）。
//! - 計算は `view3d::shape_gizmo`（始まりの形とポインタから毎回計算する）。ここは文書への入れ方と、3D ビューへの重ね描きだけ。

use egui::{pos2, vec2, Color32, Pos2, Rect, Shape as EguiShape, Stroke, Ui};
use yolu_core::fill_image::ProjectionMode;
use yolu_core::generator::Kind as GeneratorKind;
use yolu_core::glam::Vec2;
use yolu_core::{Channel, EffectSettings, FilterId, LayerId, LayerKind, RulerId};

use crate::notice::Source as NoticeSource;
use crate::state::AppState;
use crate::view3d::shape_gizmo::{self as sg, Handle, Root, Shape, Snap};

/// ギズモが動かしているもの。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// 塗りつぶしレイヤーの投影の置き場。
    Projection(LayerId),
    /// 塗りつぶしレイヤーのチャンネルのグラデーションの形。
    Gradient(LayerId, Channel),
    /// レイヤー（かマスク）のフィルターのスタックにある、形のグラデーションの Generator の形か、画像の Generator の投影の置き場。
    Filter(LayerId, FilterId),
    /// レイヤーが持つ 3D の定規（編集のモードで選んだ定規。移動の矢印と回す輪、端の点の四角。大きさのつまみは無い）。
    Ruler(LayerId, RulerId),
}

impl Target {
    pub fn layer(self) -> LayerId {
        match self {
            Target::Projection(l)
            | Target::Gradient(l, _)
            | Target::Filter(l, _)
            | Target::Ruler(l, _) => l,
        }
    }
}

/// ドラッグを始めた入力。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Mouse,
    Pen(u32),
}

/// ギズモのドラッグの途中。
#[derive(Clone, Debug, PartialEq)]
pub struct ShapeDrag {
    pub handle: Handle,
    pub start: Shape,
    /// 押した点（3D ビューの表示域の左上から）。
    pub from: Vec2,
    /// 定規の端の点のつまみを押したとき、押した点から端の四角の中心までのずれ（ドラッグの途中、ポインタにこのずれを足した所へ端を置く）。ほかは 0。
    pub grab: Vec2,
    pub target: Target,
    pub source: Source,
    /// ドラッグを始めた文書（テクスチャセットを替えたら、別の文書の同じ番号のレイヤーへ当てない）。
    pub doc_id: u128,
}

/// 形はモデルのルートの空間にあり、ルートはモデルの空間の原点・回転なし。
pub fn root() -> Root {
    Root::default()
}

fn local(rect: Rect, p: Pos2) -> Vec2 {
    Vec2::new(p.x - rect.left(), p.y - rect.top())
}

/// いまギズモを出す対象（無ければ `None`）。3D のモデルが無ければ出さない。形のグラデーションの Generator（フィルターの欄で「3D ビューで
/// 編集」にしたもの）が先で、続けて塗りつぶしのグラデーション、投影の置き場。マスクを編集している間は Generator 以外は出さない。
pub fn target(app: &AppState) -> Option<Target> {
    // 編集のモードは、点の印で選んだ物（G/R/S の途中・Q で隠したときは出さない）。ポーズのモードは出さない
    match app.mode {
        crate::mode::EditorMode::Edit => return crate::objects::gizmo_target(app),
        crate::mode::EditorMode::Pose => return None,
        crate::mode::EditorMode::Paint => {}
    }
    app.view3d.model.as_ref()?;
    let id = app.selected_layer?;
    let layer = app.doc.layer(id)?;
    if let Some((l, f)) = app.fillfx.edit_filter {
        if l == id && filter_shape(app, l, f).is_some() {
            return Some(Target::Filter(l, f));
        }
    }
    if app.m2.edit_mask {
        return None;
    }
    if layer.kind() != LayerKind::Fill {
        return None;
    }
    if let Some((l, ch)) = app.fillfx.edit_gradient {
        if l == id && layer.fill_gradient(ch).is_some() {
            return Some(Target::Gradient(l, ch));
        }
    }
    if app.fillfx.handles_hidden {
        return None;
    }
    (layer.projection().mode != ProjectionMode::Uv).then_some(Target::Projection(id))
}

/// レイヤーのフィルターのスタックの段が、形のグラデーションの Generator ならその形、UV 以外の投影の画像の Generator なら投影の置き場。
fn filter_shape(app: &AppState, layer: LayerId, filter: FilterId) -> Option<Shape> {
    let (owner, effect, _) = app.doc.find_filter(filter)?;
    if owner != layer {
        return None;
    }
    let g = effect.settings().generator_settings()?;
    match g.kind {
        GeneratorKind::ShapeGradient => Some(Shape::from_volume(&g.volume)),
        GeneratorKind::Image if g.image.projection.mode != ProjectionMode::Uv => {
            let p = &g.image.projection;
            Some(Shape::from_placement(
                &p.placement,
                p.mode == ProjectionMode::Spherical,
            ))
        }
        _ => None,
    }
}

/// 対象の今の形。
pub fn shape(app: &AppState, target: Target) -> Option<Shape> {
    let layer = app.doc.layer(target.layer())?;
    match target {
        Target::Projection(_) => {
            let p = layer.projection();
            Some(Shape::from_placement(
                &p.placement,
                p.mode == ProjectionMode::Spherical,
            ))
        }
        Target::Gradient(_, ch) => layer
            .fill_gradient(ch)
            .map(|g| Shape::from_volume(&g.volume)),
        Target::Filter(l, f) => filter_shape(app, l, f),
        Target::Ruler(l, id) => {
            crate::rulers::edit3d::gizmo_shape(&crate::objects::ruler_of(app, l, id)?)
        }
    }
}

/// ポインタの下のハンドル（定規の端の点の四角が先、そのあとは形のハンドル）。
fn hit_at(
    app: &AppState,
    t: Target,
    s: &Shape,
    view: &yolu_core::geometry::CameraView,
    p: Vec2,
) -> Handle {
    if let Target::Ruler(l, id) = t {
        if let Some(r) = crate::objects::ruler_of(app, l, id) {
            match crate::rulers::edit3d::end_at(&r, view, p) {
                Some(crate::rulers::edit3d::End::A) => return Handle::EndA,
                Some(crate::rulers::edit3d::End::B) => return Handle::EndB,
                None => {}
            }
        }
    }
    sg::hit(s, &root(), view, p)
}

/// ポインタの下のハンドル（ギズモが出ていなければ `None`）。
pub fn handle_at(app: &AppState, rect: Rect, at: Pos2) -> Handle {
    let Some(t) = target(app) else {
        return Handle::None;
    };
    let Some(s) = shape(app, t) else {
        return Handle::None;
    };
    let view = app.view3d.camera.view(rect.width(), rect.height());
    hit_at(app, t, &s, &view, local(rect, at))
}

/// ハンドルの画面の点（画面の座標。試験が掴む位置に使う）。
pub fn handle_point(app: &AppState, rect: Rect, handle: Handle) -> Option<Pos2> {
    let t = target(app)?;
    let s = shape(app, t)?;
    let view = app.view3d.camera.view(rect.width(), rect.height());
    if let (Target::Ruler(l, id), Handle::EndA | Handle::EndB) = (t, handle) {
        let r = crate::objects::ruler_of(app, l, id)?;
        let end = if handle == Handle::EndA {
            crate::rulers::edit3d::End::A
        } else {
            crate::rulers::edit3d::End::B
        };
        let p = crate::rulers::edit3d::ends(&r)
            .into_iter()
            .find(|(e, _)| *e == end)?
            .1;
        let g = crate::rulers::edit3d::end_screen(&view, p)?;
        return Some(pos2(rect.left() + g.x, rect.top() + g.y));
    }
    sg::handle_points(&s, &root(), &view)
        .into_iter()
        .find(|(h, _)| *h == handle)
        .map(|(_, p)| pos2(rect.left() + p.x, rect.top() + p.y))
}

/// 左ボタン（かペンの接触）を押した: ハンドルの上ならドラッグを始めて true（ハンドルの無い所は false で、今のツールへ）。
pub fn press(app: &mut AppState, rect: Rect, at: Pos2, source: Source) -> bool {
    if app.is_stroking() || app.fillfx.drag.is_some() {
        return false;
    }
    let Some(t) = target(app) else {
        return false;
    };
    let Some(start) = shape(app, t) else {
        return false;
    };
    let view = app.view3d.camera.view(rect.width(), rect.height());
    let p = local(rect, at);
    let handle = hit_at(app, t, &start, &view, p);
    if handle == Handle::None {
        return false;
    }
    if let Some(reason) = app.read_only_reason() {
        app.refuse(
            NoticeSource::FillLayer,
            crate::lang::refusals::read_only_set(app.lang, reason),
        );
        return true; // ハンドルを押したので、下のツールで描き始めない
    }
    app.doc.end_coalescing(); // 前の欄のドラッグにまとめない
    let grab = crate::rulers::edit3d::grab_offset(app, &view, t, handle, p);
    app.fillfx.drag = Some(ShapeDrag {
        handle,
        start,
        from: p,
        grab,
        target: t,
        source,
        doc_id: app.doc.id(),
    });
    true
}

/// ドラッグの途中（`symmetric` は Shift、`snap` は Ctrl）。始まりの形とポインタから毎回計算して、1 回の Undo にまとめて入れる。
pub fn drag_to(app: &mut AppState, rect: Rect, at: Pos2, symmetric: bool, snap: bool) {
    let Some(d) = app.fillfx.drag.clone() else {
        return;
    };
    // テクスチャセットが替わった: 前の文書のドラッグは、ここでは何もせずに捨てる（替えるときに前の文書の変更は確定済み）
    if app.doc.id() != d.doc_id {
        app.fillfx.drag = None;
        return;
    }
    // レイヤーが替わった・無くなった: そこで終える
    if app.selected_layer != Some(d.target.layer()) || app.doc.layer(d.target.layer()).is_none() {
        release(app, true);
        return;
    }
    if matches!(d.handle, Handle::EndA | Handle::EndB) {
        crate::rulers::edit3d::drag_end(app, rect, &d, at);
        return;
    }
    let view = app.view3d.camera.view(rect.width(), rect.height());
    let next = sg::drag(
        d.handle,
        &d.start,
        &root(),
        &view,
        d.from,
        local(rect, at),
        symmetric,
        snap,
        Snap::default(),
    );
    if Some(next) == shape(app, d.target) {
        return;
    }
    if let Some(Err(e)) = write_shape(app, d.target, next, true) {
        app.notify(
            crate::notice::Kind::of_core(&e),
            NoticeSource::FillLayer,
            app.lang.core_error(&e),
        );
        release(app, false);
    }
}

/// 対象の形を文書へ入れる（`coalesce` なら続けて変える操作として 1 回の Undo にまとめる）。値が範囲外なら理由を知らせて None
/// （文書は変えない）。対象が無ければ None。入れたら（変わっていれば「変更あり」の印を付けて）Some(Ok)、core が断ったら Some(Err)。
pub fn write_shape(
    app: &mut AppState,
    target: Target,
    next: Shape,
    coalesce: bool,
) -> Option<Result<(), yolu_core::CoreError>> {
    let revision = app.doc.revision();
    let result = match target {
        Target::Projection(layer) => {
            let mut p = *app.doc.layer(layer)?.projection();
            p.placement = next.into_placement();
            if let Err(e) = p.validate() {
                app.fail(NoticeSource::FillLayer, app.lang.fill_error(&e));
                return None;
            }
            app.doc.set_fill_projection(layer, p, coalesce)
        }
        Target::Gradient(layer, ch) => {
            let mut g = app
                .doc
                .layer(layer)
                .and_then(|l| l.fill_gradient(ch))
                .cloned()?;
            g.volume = next.into_volume(&g.volume);
            if g.validate().is_err() {
                app.fail(
                    NoticeSource::FillLayer,
                    app.lang
                        .pick("形の値が範囲外です", "The shape is out of range"),
                );
                return None;
            }
            app.doc.set_fill_gradient(layer, ch, Some(g), coalesce)
        }
        Target::Filter(layer, filter) => {
            let mut g = app
                .doc
                .find_filter(filter)
                .and_then(|(_, e, _)| e.settings().generator_settings().cloned())?;
            if g.kind == GeneratorKind::Image {
                g.image.projection.placement = next.into_placement();
            } else {
                g.volume = next.into_volume(&g.volume);
            }
            if g.validate().is_err() {
                app.fail(
                    NoticeSource::FillLayer,
                    app.lang
                        .pick("形の値が範囲外です", "The shape is out of range"),
                );
                return None;
            }
            app.doc
                .set_filter_settings(layer, filter, EffectSettings::generator(g), coalesce)
        }
        Target::Ruler(layer, id) => {
            let current = crate::objects::ruler_of(app, layer, id)?;
            let moved = crate::rulers::edit3d::ruler_after_shape(&current, &next);
            if moved.validate().is_err() {
                app.fail(
                    NoticeSource::Ruler,
                    app.lang
                        .pick("定規の値が範囲外です", "The ruler is out of range"),
                );
                return None;
            }
            app.ruler_put(layer, moved, coalesce)
        }
    };
    if result.is_ok() && app.doc.revision() != revision {
        app.modified = true;
    }
    Some(result)
}

/// ドラッグを終える。`commit` なら 1 回の Undo にまとめて確定、そうでなければ（Esc・フォーカスの喪失）ドラッグの前に戻して履歴にも残さない。
pub fn release(app: &mut AppState, commit: bool) {
    let Some(d) = app.fillfx.drag.take() else {
        return;
    };
    if d.doc_id != app.doc.id() {
        return; // 前の文書のドラッグ（替えるときに確定済み）。今の文書には触らない
    }
    if commit {
        app.doc.end_coalescing();
        return;
    }
    match app.doc.cancel_coalescing() {
        Ok(true) => {
            let (source, text) = if matches!(d.target, Target::Ruler(..)) {
                (
                    NoticeSource::Ruler,
                    app.lang
                        .pick("定規の操作をやめました", "Ruler edit cancelled"),
                )
            } else {
                (
                    NoticeSource::FillLayer,
                    app.lang
                        .pick("形の操作をやめました", "Shape edit cancelled"),
                )
            };
            app.info(source, text);
        }
        Ok(false) => {}
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

/// ドラッグの途中か。
pub fn dragging(app: &AppState) -> bool {
    app.fillfx.drag.is_some()
}

/// 形の線とハンドルを 3D ビューへ重ねる。`pointer` はポインタ（掴める所を光らせる）。返すのは掴める（ドラッグ中ならそれ）ハンドル。
pub fn draw(ui: &Ui, app: &mut AppState, rect: Rect, pointer: Option<Pos2>) -> Handle {
    let Some(t) = target(app) else {
        app.fillfx.hover = Handle::None;
        // 出さなくなった（レイヤーを替えた・隠した）ドラッグは取り残さない
        if app.fillfx.drag.is_some() {
            release(app, false);
        }
        return Handle::None;
    };
    // ドラッグ中に対象が替わったら終える（描く前に）
    if app.fillfx.drag.as_ref().is_some_and(|d| d.target != t) {
        release(app, false);
    }
    let Some(s) = shape(app, t) else {
        return Handle::None;
    };
    let view = app.view3d.camera.view(rect.width(), rect.height());
    let hover = match &app.fillfx.drag {
        Some(d) => d.handle,
        None => pointer
            .filter(|_| !app.is_stroking())
            .map_or(Handle::None, |p| hit_at(app, t, &s, &view, local(rect, p))),
    };
    app.fillfx.hover = hover;
    let painter = ui.painter_at(rect);
    let to = |p: Vec2| pos2(rect.left() + p.x, rect.top() + p.y);
    for line in sg::lines(&s, &root(), &view, hover) {
        let points: Vec<Pos2> = line.points.iter().map(|p| to(*p)).collect();
        if line.filled {
            painter.add(EguiShape::convex_polygon(points, line.color, Stroke::NONE));
            continue;
        }
        // 明るい面の上でも見える縁
        painter.add(EguiShape::line(
            points.clone(),
            Stroke::new(line.width + 2.0, Color32::from_black_alpha(115)),
        ));
        painter.add(EguiShape::line(points, Stroke::new(line.width, line.color)));
    }
    for (handle, at) in sg::handle_points(&s, &root(), &view) {
        let c = to(at);
        if handle == Handle::MoveFree {
            let r = Rect::from_center_size(c, vec2(sg::CENTER_POINTS, sg::CENTER_POINTS));
            painter.rect_stroke(
                r,
                0.0,
                Stroke::new(3.0, Color32::from_black_alpha(150)),
                egui::StrokeKind::Middle,
            );
            painter.rect_stroke(
                r,
                0.0,
                Stroke::new(
                    1.5,
                    if hover == handle {
                        sg::HOVER
                    } else {
                        Color32::WHITE
                    },
                ),
                egui::StrokeKind::Middle,
            );
        } else if handle.is_size() {
            let r = Rect::from_center_size(c, vec2(sg::KNOB_POINTS, sg::KNOB_POINTS));
            painter.rect_filled(r.expand(1.0), 0.0, Color32::from_black_alpha(180));
            painter.rect_filled(r, 0.0, sg::knob_color(handle, hover));
        }
    }
    hover
}

/// ハンドルを押す・ドラッグするあいだのカーソル。
pub fn cursor(app: &AppState) -> Option<egui::CursorIcon> {
    if app.fillfx.drag.is_some() {
        return Some(egui::CursorIcon::Grabbing);
    }
    (app.fillfx.hover != Handle::None).then_some(egui::CursorIcon::Grab)
}
