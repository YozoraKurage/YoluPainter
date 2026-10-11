//! 3D の回転中心・自動深度・ポインタへの拡縮。押したときのモデルとカメラを離すまで保持する。
//!
//! 透視と正投影: 視点のパイ・軸の印の真ん中で切り替える（[`toggle_orthographic`]）。軸の向きで正投影の設定が入なら、軸の視点を選んだとき・
//! スナップ回転で軸に吸い付いたときに正投影にし（[`enter_axis`]）、回して軸から外れたら元の透視へ戻す（[`after_orbit`]）。手で切り替えた
//! 投影は、軸から外れても戻さない。

use std::sync::Arc;

use egui::{Modifiers, Pos2, Rect, Ui};
use yolu_core::geometry::{
    aligned_axis_view, orbited, pick, snap_orientation, AxisView, Bounds, OrbitCamera,
};
use yolu_core::glam::{Vec2, Vec3};

use super::{model::ViewModel, Nav, View3dState};
use crate::{lang::Lang, settings::Problem, state::AppState};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OrbitCenter {
    #[default]
    View,
    Surface,
    Model,
    TextureSet,
}

impl OrbitCenter {
    pub const ALL: [Self; 4] = [Self::View, Self::Surface, Self::Model, Self::TextureSet];

    pub fn label(self, lang: Lang) -> &'static str {
        match self {
            Self::View => lang.pick("画面の中心", "View center"),
            Self::Surface => lang.pick("面の位置（自動深度）", "Surface (auto depth)"),
            Self::Model => lang.pick("モデルの中心", "Model center"),
            Self::TextureSet => lang.pick("テクスチャセットの中心", "Texture set center"),
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::View => "view",
            Self::Surface => "surface",
            Self::Model => "model",
            Self::TextureSet => "texture_set",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ZoomCenter {
    #[default]
    View,
    Pointer,
}

impl ZoomCenter {
    pub const ALL: [Self; 2] = [Self::View, Self::Pointer];

    pub fn label(self, lang: Lang) -> &'static str {
        match self {
            Self::View => lang.pick("画面の中心へ", "Toward view center"),
            Self::Pointer => lang.pick("ポインタの所へ", "Toward pointer"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Preferences {
    pub orbit: OrbitCenter,
    pub zoom: ZoomCenter,
    /// 軸の向きで正投影（軸の視点・スナップ回転で軸に入ったら正投影にし、外れたら戻す）。
    pub axis_ortho: bool,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            orbit: OrbitCenter::default(),
            zoom: ZoomCenter::default(),
            axis_ortho: true,
        }
    }
}

impl Preferences {
    pub fn parse(&mut self, key: &str, value: &str, problems: &mut Vec<Problem>) {
        match key {
            "view3d_orbit" => match OrbitCenter::ALL.into_iter().find(|c| c.key() == value) {
                Some(c) => self.orbit = c,
                None => problems.push(Problem::Invalid {
                    key: "view3d_orbit",
                    value: value.into(),
                }),
            },
            "view3d_zoom" => match value {
                "view" => self.zoom = ZoomCenter::View,
                "pointer" => self.zoom = ZoomCenter::Pointer,
                _ => problems.push(Problem::Invalid {
                    key: "view3d_zoom",
                    value: value.into(),
                }),
            },
            "view3d_axis_ortho" => match value {
                "on" => self.axis_ortho = true,
                "off" => self.axis_ortho = false,
                _ => problems.push(Problem::Invalid {
                    key: "view3d_axis_ortho",
                    value: value.into(),
                }),
            },
            _ => {}
        }
    }

    pub fn write(self, text: &mut String) {
        if self.orbit != OrbitCenter::View {
            text.push_str(&format!("view3d_orbit={}\n", self.orbit.key()));
        }
        if self.zoom == ZoomCenter::Pointer {
            text.push_str("view3d_zoom=pointer\n");
        }
        if !self.axis_ortho {
            text.push_str("view3d_axis_ortho=off\n");
        }
    }
}

/// 表示中の、今のテクスチャセットの面の境界。別のセット・隠した面は含まない。
pub fn selected_bounds(state: &View3dState) -> Option<Bounds> {
    material_bounds(state.model.as_deref()?, state.material)
}

fn material_bounds(model: &ViewModel, material: i32) -> Option<Bounds> {
    if material < 0 {
        return None;
    }
    let mut points = model
        .geometry
        .triangles()
        .iter()
        .filter(|t| t.material == material)
        .flat_map(|t| [t.a, t.b, t.c]);
    let mut bounds = Bounds::point(points.next()?);
    for p in points {
        bounds.encapsulate_point(p);
    }
    Some(bounds)
}

fn local(rect: Rect, at: Pos2) -> Vec2 {
    Vec2::new(at.x - rect.left(), at.y - rect.top())
}

fn point_under(
    model: Option<&ViewModel>,
    camera: OrbitCamera,
    rect: Rect,
    at: Pos2,
) -> Option<Vec3> {
    pick(
        &model?.geometry,
        &camera.view(rect.width(), rect.height()),
        local(rect, at),
    )
    .map(|hit| hit.position)
}

/// 面が無い所では注視点と同じ深さの平面へ落とす。
fn zoom_point(model: Option<&ViewModel>, camera: OrbitCamera, rect: Rect, at: Pos2) -> Vec3 {
    point_under(model, camera, rect, at).unwrap_or_else(|| {
        camera
            .view(rect.width(), rect.height())
            .point_at_depth(local(rect, at), camera.distance)
    })
}

pub struct Drag {
    at: Pos2,
    /// 回すドラッグが、押した所から遊び（クリックとみなす距離）を超えて動いたか（超えるまで回さない。`rotation_delta`）。
    moved: bool,
    camera: OrbitCamera,
    /// スナップ回転の、吸い付ける前の向き（yaw・pitch）。吸い付いた向きを元に回すと、いちど吸い付いたら離れられないので、
    /// 回した分はこちらへ溜めて、カメラには吸い付けた結果を当てる。
    free: Option<(f32, f32)>,
    model: Option<Arc<ViewModel>>,
    model_center: Option<Vec3>,
    material: i32,
    preferences: Preferences,
    resolved: Option<Anchor>,
}

#[derive(Clone, Copy)]
struct Anchor {
    pivot: Vec3,
    surface: Option<Vec3>,
    zoom: Vec3,
}

impl Drag {
    pub fn new(state: &View3dState, preferences: Preferences, at: Pos2) -> Self {
        Self {
            at,
            moved: false,
            camera: state.camera,
            free: None,
            model: state.model.clone(),
            model_center: state.full_model().map(|m| m.geometry.bounds().center),
            material: state.material,
            preferences,
            resolved: None,
        }
    }

    /// 回すドラッグ（右ボタン・Alt + 左）で、このポインタの位置を受けて回す量（画面の点）。押した所から `dead_zone` までは回さない（動かさずに離す
    /// 操作 — スポイト・クローンの元 — が、小さな揺れで視点を動かして別の点を指さないように）。超えた最初の動きでは、押した所からの動きを全部当てる。
    /// `previous` は前の位置。
    pub fn rotation_delta(
        &mut self,
        pos: Pos2,
        previous: Pos2,
        dead_zone: f32,
    ) -> Option<egui::Vec2> {
        if self.moved {
            return Some(pos - previous);
        }
        if self.at.distance(pos) <= dead_zone {
            return None;
        }
        self.moved = true;
        Some(pos - self.at)
    }

    fn anchor(&mut self, rect: Rect) -> Anchor {
        if let Some(anchor) = self.resolved {
            return anchor;
        }
        let hit = (self.preferences.orbit == OrbitCenter::Surface)
            .then(|| point_under(self.model.as_deref(), self.camera, rect, self.at))
            .flatten();
        let pivot = match self.preferences.orbit {
            OrbitCenter::View => None,
            OrbitCenter::Surface => hit,
            OrbitCenter::Model => self.model_center,
            OrbitCenter::TextureSet => self
                .model
                .as_deref()
                .and_then(|m| material_bounds(m, self.material))
                .map(|b| b.center),
        }
        .unwrap_or(self.camera.target);
        let zoom = if self.preferences.zoom == ZoomCenter::Pointer {
            zoom_point(self.model.as_deref(), self.camera, rect, self.at)
        } else {
            self.camera.target
        };
        let anchor = Anchor {
            pivot,
            surface: hit,
            zoom,
        };
        self.resolved = Some(anchor);
        anchor
    }
}

pub fn move_by(app: &mut AppState, rect: Rect, nav: Nav, dx: f32, dy: f32) {
    let Some(drag) = app.view3d.input.navigation.as_mut() else {
        return;
    };
    let anchor = drag.anchor(rect);
    let preferences = drag.preferences;
    let camera = &mut app.view3d.camera;
    match nav {
        Nav::Orbit if preferences.orbit != OrbitCenter::View => {
            camera.orbit_about(anchor.pivot, dx, dy);
            after_orbit(&mut app.view3d, anchor.pivot);
        }
        Nav::Orbit => {
            camera.orbit(dx, dy);
            let target = camera.target;
            after_orbit(&mut app.view3d, target);
        }
        Nav::SnapOrbit => {
            let (yaw, pitch) = drag.free.unwrap_or((camera.yaw, camera.pitch));
            let free = orbited(yaw, pitch, dx, dy);
            drag.free = Some(free);
            let (yaw, pitch) = snap_orientation(free.0, free.1);
            let pivot = match preferences.orbit {
                OrbitCenter::View => camera.target,
                _ => anchor.pivot,
            };
            camera.set_orientation_about(pivot, yaw, pitch);
            if aligned_axis_view(yaw, pitch).is_some() {
                enter_axis(&mut app.view3d, preferences, pivot);
            } else {
                after_orbit(&mut app.view3d, pivot);
            }
        }
        Nav::Pan if preferences.orbit == OrbitCenter::Surface => {
            // ホイールを挟んでも、押した面での画面上の移動量を保つ。
            let depth = anchor
                .surface
                .map(|p| (p - camera.position()).dot(camera.rotation() * Vec3::Z))
                .filter(|d| *d > 0.0)
                .unwrap_or(camera.distance);
            camera.pan_at_depth(dx, dy, rect.height(), depth)
        }
        Nav::Pan => camera.pan(dx, dy, rect.height()),
        Nav::Zoom if preferences.zoom == ZoomCenter::Pointer => {
            camera.zoom_towards(anchor.zoom, dx)
        }
        Nav::Zoom => camera.zoom(dx),
    }
}

/// 右ボタン（ペンのサイドボタンも）を押して視点を回している間か。この間だけ、W/A/S/D/Q/E は視点の移動（`fly`）。
pub fn flying(app: &AppState) -> bool {
    matches!(
        app.view3d.input.nav,
        Some((Nav::Orbit, egui::PointerButton::Secondary))
    )
}

/// 視点の移動の速さ（1 秒あたり、モデルの半径のこの倍）と、Shift を押したときの倍率。
pub const FLY_SPEED: f32 = 0.5;
pub const FLY_FAST: f32 = 3.0;
/// 1 フレームの長さの上限（秒。止まったあとの最初のフレームで飛ばない）。
const FLY_MAX_DT: f32 = 0.1;

/// 右ボタンを押している間、W/S（前後）・A/D（左右）・Q/E（下上）を押していれば、注視点とカメラを一緒に動かす（距離は変えない。
/// Shift で速く）。毎フレームの長さ（`stable_dt`）で動かし、動かしている間は描き直しを頼む。動かしたら、右ボタンを動かさずに離してもスポイトにしない。
pub fn fly(ui: &Ui, app: &mut AppState) {
    if !flying(app) || app.is_stroking() || ui.ctx().egui_wants_keyboard_input() {
        return;
    }
    let (direction, dt, fast) = ui.input(|i| {
        let mut direction = Vec3::ZERO;
        if !i.modifiers.command && !i.modifiers.ctrl && !i.modifiers.alt {
            for key in crate::keymap::FLY_KEYS {
                if crate::keymap::hold_down(i, key.command) {
                    direction += Vec3::from(key.direction);
                }
            }
        }
        (direction, i.stable_dt, i.modifiers.shift)
    });
    let direction = direction.normalize_or_zero();
    if direction == Vec3::ZERO {
        return;
    }
    let speed = app.view3d.camera.model_radius * FLY_SPEED * if fast { FLY_FAST } else { 1.0 };
    app.view3d
        .camera
        .fly(direction * speed * dt.min(FLY_MAX_DT));
    app.view3d.input.eyedrop = None;
    ui.ctx().request_repaint();
}

pub fn wheel(app: &mut AppState, rect: Rect, at: Pos2, notches: f32) {
    if app.prefs.settings.navigation.zoom == ZoomCenter::Pointer {
        let point = zoom_point(app.view3d.model.as_deref(), app.view3d.camera, rect, at);
        app.view3d.camera.zoom_towards(point, notches);
    } else {
        app.view3d.camera.zoom(notches);
    }
}

fn can_frame(app: &AppState) -> bool {
    !app.is_stroking()
        && !app.stencil.handling()
        && app.view3d.input.nav.is_none()
        && app.view3d.pose.drag.is_none()
        && app.fillfx.drag.is_none()
        && app.path.drag.is_none()
        && app.region.drag.is_none()
}

/// 視点の操作（パイ・メニューから。キーの表の操作 `view3d.view_*`・`view3d.frame_selected`・`view3d.ortho`）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NavOp {
    /// 軸の視点（正面・背面・右・左・上・下）にする。
    Axis(AxisView),
    /// 選んだセットを収める（3D ビューの上の `.` と同じ）。
    FrameSelected,
    /// 透視と正投影を切り替える。
    ToggleOrthographic,
}

/// 視点の操作を当てる（`Action::View3dNav`）。描いている・視点を動かしている・ギズモをドラッグしている間は何もしない（`.` と同じ）。
pub fn apply(app: &mut AppState, op: NavOp) {
    match op {
        NavOp::Axis(view) => axis_view(app, view),
        NavOp::FrameSelected => {
            if let Some(rect) = app.view3d.view_rect {
                frame_selected(app, rect);
            }
        }
        NavOp::ToggleOrthographic => toggle_orthographic(app),
    }
}

/// 視点を動かせるか（モデルがあり、描いている・視点やギズモを動かしている途中でない）。
pub fn can_move_view(app: &AppState) -> bool {
    app.view3d.model.is_some() && can_frame(app)
}

/// 押した所の無い向きの替え（軸の視点・軸の印のドラッグ）で回す中心: 回す中心の設定がモデルの中心・テクスチャセットの中心ならその点、
/// ほか（画面の中心・面の位置）は注視点。
pub fn pivot_without_press(app: &AppState) -> yolu_core::glam::Vec3 {
    match app.prefs.settings.navigation.orbit {
        OrbitCenter::View | OrbitCenter::Surface => None,
        OrbitCenter::Model => app.view3d.full_model().map(|m| m.geometry.bounds().center),
        OrbitCenter::TextureSet => selected_bounds(&app.view3d).map(|b| b.center),
    }
    .unwrap_or(app.view3d.camera.target)
}

/// 軸の視点にする。上・下も含めて画面の向きは `AxisView::orientation` のとおり（yaw は今の値から何周しているかを保つ）。回す中心の設定が
/// モデルの中心・テクスチャセットの中心なら、その点の画面の位置を変えない（面の位置の設定は押した所が無いので、注視点）。軸の向きで
/// 正投影の設定が入なら正投影にする。
pub fn axis_view(app: &mut AppState, view: AxisView) {
    if !can_move_view(app) {
        return;
    }
    let camera = app.view3d.camera;
    let (axis_yaw, pitch) = view.orientation();
    let yaw = axis_yaw + 360.0 * ((camera.yaw - axis_yaw) / 360.0).round();
    let pivot = pivot_without_press(app);
    app.view3d.camera.set_orientation_about(pivot, yaw, pitch);
    let preferences = app.prefs.settings.navigation;
    enter_axis(&mut app.view3d, preferences, pivot);
}

/// 透視と正投影を切り替える（回す中心（設定がモデル・テクスチャセットの中心ならその点、ほかは注視点）の画面の位置と大きさは変えない。
/// 軸の向きで自動に替えるときと同じ中心）。手で選んだ投影は、軸から外れても戻さない。
pub fn toggle_orthographic(app: &mut AppState) {
    if !can_move_view(app) {
        return;
    }
    let on = !app.view3d.camera.is_orthographic();
    let pivot = pivot_without_press(app);
    app.view3d.camera.set_orthographic_about(on, pivot);
    app.view3d.auto_orthographic = false;
}

/// 軸の向きに入った（軸の視点を選んだ・スナップ回転で軸に吸い付いた）: 軸の向きで正投影の設定が入で、今が透視なら正投影にし、
/// 軸から外れたら戻す印を付ける。回す中心 pivot の画面の位置と大きさは変えない。
pub fn enter_axis(state: &mut View3dState, preferences: Preferences, pivot: Vec3) {
    if preferences.axis_ortho && !state.camera.is_orthographic() {
        state.camera.set_orthographic_about(true, pivot);
        state.auto_orthographic = true;
    }
}

/// 向きを変えた後: 軸の向きに入って自動で正投影にしていて、軸の向きから外れたら透視へ戻す（回す中心 pivot の画面の位置と大きさは
/// 変えない）。
pub fn after_orbit(state: &mut View3dState, pivot: Vec3) {
    if state.auto_orthographic && aligned_axis_view(state.camera.yaw, state.camera.pitch).is_none()
    {
        state.camera.set_orthographic_about(false, pivot);
        state.auto_orthographic = false;
    }
}

pub fn frame_selected(app: &mut AppState, rect: Rect) {
    if !can_frame(app) {
        return;
    }
    if let Some(bounds) = selected_bounds(&app.view3d) {
        app.view3d
            .camera
            .frame_bounds(&bounds, rect.width(), rect.height());
    }
}

/// 修飾なしの . は、3D の上でだけ選んだセットを収める。文字入力やダイアログには渡したままにする。
pub fn shortcut(ui: &Ui, app: &mut AppState, rect: Rect, foreign: bool) {
    let ctx = ui.ctx();
    if foreign
        || ctx.egui_wants_keyboard_input()
        || app.popup.is_some()
        || app.ui.popup_was_open
        || app.view3d.display.settings_open
        || app.dock_grabbed()
        || app.sel.dialog.is_some()
        || crate::windows::modal_open(app)
        || !can_frame(app)
    {
        return;
    }
    let over = ui.input(|i| i.pointer.hover_pos()).is_some_and(|p| {
        rect.contains(p)
            && ctx
                .layer_id_at(p)
                .is_none_or(|layer| layer == ui.layer_id())
    });
    if over
        && ui.input(|i| i.modifiers == Modifiers::NONE)
        && ctx.input_mut(|i| crate::keymap::consume_command(i, app, "view3d.frame_selected"))
    {
        frame_selected(app, rect);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Action;

    #[test]
    fn the_projection_and_the_axis_views_do_not_move_while_drawing_or_without_a_model() {
        let mut app = AppState::new(32, 32);
        // モデルが無い: 何もしない
        let before = app.view3d.camera;
        app.apply(Action::View3dNav(NavOp::ToggleOrthographic));
        app.apply(Action::View3dNav(NavOp::Axis(AxisView::Top)));
        assert_eq!(app.view3d.camera, before);
        // 描いている間: カメラを動かさない（区画の投影の画素を覚えて使うので）
        app.view3d.load_demo();
        let layer = app.selected_layer.unwrap();
        let settings = app.stroke_settings(false);
        let stroke = app.doc.begin_stroke(layer, &settings).unwrap();
        app.stroke = Some(stroke);
        app.canvas.stroke = Some(crate::state::StrokeSource::Mouse);
        let before = app.view3d.camera;
        app.apply(Action::View3dNav(NavOp::ToggleOrthographic));
        app.apply(Action::View3dNav(NavOp::Axis(AxisView::Top)));
        assert_eq!(app.view3d.camera, before);
        assert!(!app.view3d.auto_orthographic);
        // 描き終えたら効く（軸の視点は軸の向きで正投影）
        let stroke = app.stroke.take().unwrap();
        app.canvas.stroke = None;
        app.doc.end_stroke(stroke).unwrap();
        app.apply(Action::View3dNav(NavOp::Axis(AxisView::Top)));
        assert!(app.view3d.camera.is_orthographic());
        assert!(app.view3d.auto_orthographic);
        assert_eq!(app.view3d.camera.pitch, 90.0);
    }

    #[test]
    fn switching_by_hand_keeps_the_orbit_center_in_place_and_size_like_the_axis_views() {
        let mut app = AppState::new(32, 32);
        app.view3d.load_demo();
        app.prefs.settings.navigation.orbit = OrbitCenter::Model;
        // パンして、注視点をモデルの中心から外す（モデルの中心は画面の中心でも注視点の奥行きでもない）
        app.view3d.camera.pan(120.0, -40.0, 480.0);
        app.view3d
            .camera
            .fly(yolu_core::glam::Vec3::new(0.0, 0.0, -0.8));
        let center = app.view3d.full_model().unwrap().geometry.bounds().center;
        let probe = center + app.view3d.camera.rotation() * Vec3::X * 0.05;
        let seen = |app: &AppState| {
            let v = app.view3d.camera.view(640.0, 480.0);
            (v.to_screen(center).unwrap(), v.to_screen(probe).unwrap())
        };
        let before = seen(&app);
        for on in [true, false] {
            app.apply(Action::View3dNav(NavOp::ToggleOrthographic));
            assert_eq!(app.view3d.camera.is_orthographic(), on);
            let now = seen(&app);
            assert!(
                (now.0 - before.0).length() < 2e-2,
                "{on}: {before:?} {now:?}"
            );
            assert!(
                (now.1 - before.1).length() < 2e-2,
                "{on}: {before:?} {now:?}"
            );
        }
        // 画面の中心のときは注視点（前と同じ）
        app.prefs.settings.navigation.orbit = OrbitCenter::View;
        let mut expected = app.view3d.camera;
        expected.set_orthographic(true);
        app.apply(Action::View3dNav(NavOp::ToggleOrthographic));
        assert_eq!(app.view3d.camera, expected);
    }
}
