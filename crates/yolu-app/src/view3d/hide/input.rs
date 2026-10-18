//! ハイドのモードの 3D ビューの左ボタン: ボーンの丸の上ならそのボーンを選ぶ（「ボーンで隠す」に使う）。丸の外は、選ぶツールで
//! UV アイランドを選ぶ（押した面のアイランド、または四角で囲んだアイランド。Shift で足す・Ctrl で外す・何も無い所を押すと選びを外す）。
//! 選ぶのは今のビューの見せる形の面だけ（隠した面・裏の面・手前の面に隠れた面は選ばない）。

use egui::{Pos2, Rect};
use yolu_core::geometry::pick;
use yolu_core::glam::Vec2;

use super::HideTool;
use crate::state::key::Modifiers;
use crate::state::AppState;

/// 四角の選びで、手前の面に隠れた面を光線で除く三角形の数の上限（これより多い形は、裏向きだけを除く）。
const OCCLUSION_LIMIT: usize = 200_000;

/// 選びの組み合わせ方。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Combine {
    /// 選び直す。
    Replace,
    /// 足す（Shift）。
    Add,
    /// 外す（Ctrl）。
    Remove,
}

impl Combine {
    pub fn of(m: &Modifiers) -> Combine {
        if m.shift {
            Combine::Add
        } else if m.ctrl || m.command {
            Combine::Remove
        } else {
            Combine::Replace
        }
    }
}

/// 四角で選んでいる途中（画面の点）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RectDrag {
    pub start: Pos2,
    pub now: Pos2,
    pub combine: Combine,
}

impl RectDrag {
    pub fn rect(&self) -> Rect {
        Rect::from_two_pos(self.start, self.now)
    }
}

fn local(rect: Rect, p: Pos2) -> Vec2 {
    Vec2::new(p.x - rect.left(), p.y - rect.top())
}

/// 画面の点の下の、今のビューの見せる形の面の UV アイランド。
pub fn island_at(app: &mut AppState, rect: Rect, at: Pos2) -> Option<u32> {
    let model = app.view3d.model.clone()?;
    let view = app.view3d.view.camera.view(rect.width(), rect.height());
    let hit = pick(&model.geometry, &view, local(rect, at))?;
    let triangle = app.view3d.full_triangle(hit.triangle)?;
    super::island_index(app)?.island_of(triangle)
}

/// 選びに組み合わせる。
fn combine(selected: &mut Vec<u32>, found: &[u32], how: Combine) {
    match how {
        Combine::Replace => {
            selected.clear();
            selected.extend_from_slice(found);
        }
        Combine::Add => {
            for i in found {
                if !selected.contains(i) {
                    selected.push(*i);
                }
            }
        }
        Combine::Remove => selected.retain(|i| !found.contains(i)),
    }
}

/// 左ボタンを押した（ハイドのモードの今のビュー）。
pub fn press(app: &mut AppState, rect: Rect, at: Pos2, modifiers: &Modifiers) {
    if app.is_stroking() {
        return;
    }
    // ボーンの丸が先
    if let Some(bone) = crate::view3d::bone_marks::bone_at(app, rect, at) {
        if let Some(s) = app.view3d.pose.session.as_mut() {
            s.selected = Some(bone);
            crate::view3d::gizmo::reveal(s, bone);
        }
        return;
    }
    let how = Combine::of(modifiers);
    match app.view3d.hiding.tool {
        HideTool::Island => {
            let found: Vec<u32> = island_at(app, rect, at).into_iter().collect();
            let selected = &mut app.view3d.view.hide.selected;
            // 押したアイランドを Shift で押し直すと外す（足す・外すを 1 つのボタンで）
            if how == Combine::Add && found.len() == 1 && selected.contains(&found[0]) {
                combine(selected, &found, Combine::Remove);
            } else {
                combine(selected, &found, how);
            }
        }
        HideTool::Rectangle => {
            app.view3d.view.hide.rect = Some(RectDrag {
                start: at,
                now: at,
                combine: how,
            });
        }
    }
}

/// ポインタが動いた（四角の選びの途中なら広げる）。
pub fn moved(app: &mut AppState, at: Pos2) {
    if let Some(r) = app.view3d.view.hide.rect.as_mut() {
        r.now = at;
    }
}

/// 離した（四角の選びを終える）。
pub fn release(app: &mut AppState, rect: Rect, at: Pos2) {
    let Some(mut drag) = app.view3d.view.hide.rect.take() else {
        return;
    };
    drag.now = at;
    let found = islands_in(app, rect, drag.rect());
    combine(&mut app.view3d.view.hide.selected, &found, drag.combine);
}

/// 四角の選びをやめる（Esc・フォーカスを失った）。
pub fn cancel(app: &mut AppState) {
    app.view3d.view.hide.rect = None;
}

/// 四角の選びの途中か。
pub fn dragging(app: &AppState) -> bool {
    app.view3d.view.hide.rect.is_some()
}

/// 画面の四角（`area`）の中に、見えている面の真ん中があるアイランド（今のビューの見せる形の面だけ。裏の面と、形が小さければ手前の面に
/// 隠れた面は除く）。
pub fn islands_in(app: &mut AppState, rect: Rect, area: Rect) -> Vec<u32> {
    let Some(model) = app.view3d.model.clone() else {
        return Vec::new();
    };
    let Some(index) = super::island_index(app) else {
        return Vec::new();
    };
    let view = app.view3d.view.camera.view(rect.width(), rect.height());
    let local_area = area.translate(-rect.min.to_vec2());
    let g = &model.geometry;
    let occlusion = g.triangle_count() <= OCCLUSION_LIMIT;
    let mut seen = vec![false; index.len()];
    let mut out = Vec::new();
    for (i, t) in g.triangles().iter().enumerate() {
        let Some(full) = app.view3d.full_triangle(i as u32) else {
            continue;
        };
        let Some(island) = index.island_of(full) else {
            continue;
        };
        if seen[island as usize] {
            continue;
        }
        let centre = (t.a + t.b + t.c) / 3.0;
        let Some(s) = view.to_screen(centre) else {
            continue;
        };
        if !local_area.contains(egui::pos2(s.x, s.y)) {
            continue;
        }
        if t.normal().dot(view.to_viewer(centre)) <= 0.0 {
            continue;
        }
        if occlusion {
            let sight = view.sight(centre);
            let distance = sight.distance;
            if let Some(hit) = g.raycast(sight.ray(), true, f32::INFINITY) {
                if hit.triangle != i as u32
                    && hit.distance < distance - distance * 1e-3 - g.visibility_epsilon()
                {
                    continue;
                }
            }
        }
        seen[island as usize] = true;
        out.push(island);
    }
    out
}

/// ポインタの下のアイランドを覚える（重ねで薄く強調する。ボーンの丸の上・回している間・四角の途中は外す）。
pub fn hover(app: &mut AppState, rect: Rect, pointer: Option<Pos2>) {
    let pointed = match pointer {
        Some(p)
            if app.view3d.view.input.nav.is_none()
                && app.view3d.view.hide.rect.is_none()
                && crate::view3d::bone_marks::bone_at(app, rect, p).is_none() =>
        {
            island_at(app, rect, p)
        }
        _ => None,
    };
    app.view3d.view.hide.pointed = pointed;
}
