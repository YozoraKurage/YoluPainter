//! ボーンの丸（ハイドとポーズのモードの 3D ビュー）: 全部のボーンの位置（ボーンの原点）に、塗りの無い半透明の輪を出す。輪の大きさはモデルの
//! 大きさに比べた世界の半径を画面へ写したもので、ズームについて 3〜9 点の間で変わる。輪を掴める範囲は見た目の半径 + 3 点。
//! 位置はそのビューのポーズ（シェルフのポーズのプリセットがあればその形、無ければ今のポーズ）で求める。

use egui::{Color32, Pos2, Rect, Stroke, Ui};
use yolu_core::glam::Vec2;
use yolu_core::skin::{Pose, Rig};

use crate::objects::SELECTED;
use crate::state::AppState;

/// 丸の世界での半径（モデルの外接の箱の対角線に対する割合。1.7 m の人で約 1 cm）。
const WORLD_FRACTION: f32 = 0.005;
/// 画面での半径の下限と上限（点）。
pub const MIN_RADIUS: f32 = 3.0;
pub const MAX_RADIUS: f32 = 9.0;
/// 掴める範囲の、見た目の半径からの広がり（点）。
pub const GRAB_MARGIN: f32 = 3.0;

/// 丸を出すモードか（ハイドとポーズ）。
pub fn shown(app: &AppState) -> bool {
    matches!(
        app.mode,
        crate::mode::EditorMode::Hide | crate::mode::EditorMode::Pose
    ) && app.view3d.pose.session.is_some()
}

/// 1 つの丸（ボーンの番号・画面の点・画面の半径）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mark {
    pub bone: usize,
    pub at: Pos2,
    pub radius: f32,
}

/// 今のビューのポーズ（シェルフのポーズのプリセットがあればそれ、無ければ今のポーズ）。
pub fn view_pose(app: &AppState) -> Option<&Pose> {
    let s = app.view3d.pose.session.as_ref()?;
    Some(app.view3d.view.pose.pose().unwrap_or(s.pose()))
}

/// 今のビューの全部のボーンの丸（カメラの後ろのボーンは除く）。
pub fn marks(app: &AppState, rect: Rect) -> Vec<Mark> {
    let Some(s) = app.view3d.pose.session.as_ref() else {
        return Vec::new();
    };
    let Some(pose) = view_pose(app) else {
        return Vec::new();
    };
    let Ok(world) = s.rig.world_matrices(pose) else {
        return Vec::new();
    };
    let view = app.view3d.view.camera.view(rect.width(), rect.height());
    let diagonal = app
        .view3d
        .full_model()
        .map_or(1.0, |m| m.geometry.bounds().extents.length() * 2.0)
        .max(1e-6);
    let world_radius = diagonal * WORLD_FRACTION;
    (0..s.rig.bones().len())
        .filter_map(|bone| {
            let p = Rig::bone_origin(&world, bone);
            let screen = view.to_screen(p)?;
            if view.world_radius_to_screen(p, 1.0) <= 0.0 {
                return None;
            }
            let radius = view
                .world_radius_to_screen(p, world_radius)
                .clamp(MIN_RADIUS, MAX_RADIUS);
            Some(Mark {
                bone,
                at: Pos2::new(rect.left() + screen.x, rect.top() + screen.y),
                radius,
            })
        })
        .collect()
}

/// 画面の点で掴める丸のボーン（見た目の半径 + 3 点の内側。重なるときは真ん中がいちばん近いもの）。
pub fn bone_at(app: &AppState, rect: Rect, at: Pos2) -> Option<usize> {
    if !shown(app) {
        return None;
    }
    marks(app, rect)
        .into_iter()
        .filter_map(|m| {
            let d = Vec2::new(m.at.x - at.x, m.at.y - at.y).length();
            (d <= m.radius + GRAB_MARGIN).then_some((d, m.bone))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, bone)| bone)
}

/// 丸を描く（選んだボーンは橙。ポインタの乗った丸は少し濃く・少し大きく）。
pub fn draw(ui: &Ui, app: &AppState, rect: Rect, pointer: Option<Pos2>) {
    if !shown(app) {
        return;
    }
    let selected = app.view3d.pose.session.as_ref().and_then(|s| s.selected);
    let hover = pointer.and_then(|p| bone_at(app, rect, p));
    let painter = ui.painter_at(rect);
    for m in marks(app, rect) {
        let hot = hover == Some(m.bone);
        let radius = if hot { m.radius + 2.0 } else { m.radius };
        let color = if selected == Some(m.bone) {
            SELECTED.gamma_multiply(if hot { 1.0 } else { 0.85 })
        } else {
            Color32::WHITE.gamma_multiply(if hot { 0.85 } else { 0.6 })
        };
        // 輪の線の太さは 2 点。黒い縁（35%）は輪の外側に 1.5 点
        painter.circle_stroke(
            m.at,
            radius + 1.75,
            Stroke::new(1.5, Color32::from_black_alpha(if hot { 120 } else { 90 })),
        );
        painter.circle_stroke(m.at, radius, Stroke::new(2.0, color));
    }
}
