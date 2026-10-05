//! 3D の回転中心・自動深度・ポインタへの拡縮。押したときのモデルとカメラを離すまで保持する。

use std::sync::Arc;

use egui::{Modifiers, Pos2, Rect, Ui};
use yolu_core::geometry::{pick, Bounds, OrbitCamera};
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

    /// ツールチップ（面の位置だけ、パンの速さも面の深さに合わせる）。
    pub fn tip(self, lang: Lang) -> &'static str {
        if self == Self::Surface {
            lang.pick(
                "回し始めの面を中心にし、パンの速さも面の深さに合わせます。面が無ければ今の中心です。",
                "Orbit around the starting surface and pan at its depth. Empty space keeps the current center.",
            )
        } else {
            lang.pick(
                "回し始めに中心を決め、離すまで保ちます。",
                "Choose the pivot at the start and keep it until release.",
            )
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

    /// ツールチップ。
    pub fn tip(self, lang: Lang) -> &'static str {
        match self {
            Self::View => lang.pick("画面の中心を保ってズームします。", "Zoom keeping the view center."),
            Self::Pointer => lang.pick(
                "面が無ければポインタの向きへ寄ります。",
                "Empty space zooms along the pointer direction.",
            ),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Preferences {
    pub orbit: OrbitCenter,
    pub zoom: ZoomCenter,
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
        let view = camera.view(rect.width(), rect.height());
        let ray = view.ray(local(rect, at));
        view.position + ray.direction() * (camera.distance / ray.direction().dot(view.forward))
    })
}

pub struct Drag {
    at: Pos2,
    camera: OrbitCamera,
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
            camera: state.camera,
            model: state.model.clone(),
            model_center: state.full_model().map(|m| m.geometry.bounds().center),
            material: state.material,
            preferences,
            resolved: None,
        }
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
            camera.orbit_about(anchor.pivot, dx, dy)
        }
        Nav::Orbit => camera.orbit(dx, dy),
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
        || app.popup_was_open
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
        && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, crate::keymap::VIEW3D_FRAME))
    {
        frame_selected(app, rect);
    }
}
