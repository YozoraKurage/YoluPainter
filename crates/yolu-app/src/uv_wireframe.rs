//! 文書とは独立した UV の辺と表示座標のキャッシュ。
mod color;
#[cfg(test)]
mod tests;

use std::{collections::HashSet, sync::Arc};

use egui::{Color32, Pos2, Stroke, Ui};
use yolu_core::{
    geometry::{SurfaceGeometry, SurfaceTriangle},
    glam::Vec2,
};

use crate::{
    canvas::view::CanvasView,
    state::{Action, AppState},
    ui::widgets as w,
};

pub const MAX_EDGES: usize = 1 << 20;
pub const DEFAULT_COLOR: [u8; 4] = [89, 217, 255, 153];

pub struct Wireframe {
    /// 辺を作った（または同じ辺になると確かめた）幾何。ポーズで世代が替わるたびに持ち替える。
    geometry: Option<Arc<SurfaceGeometry>>,
    material: i32,
    /// 辺の上限。通常は `MAX_EDGES`。試験が下げて、超過の経路を確かめる。
    limit: usize,
    edges: Vec<[Vec2; 2]>,
    truncated: bool,
    view: Option<CanvasView>,
    screen: Vec<[Pos2; 2]>,
    /// 辺を作った回数（試験用）。
    #[cfg(test)]
    builds: u32,
}

impl Default for Wireframe {
    fn default() -> Self {
        Wireframe {
            geometry: None,
            material: -1,
            limit: MAX_EDGES,
            edges: Vec::new(),
            truncated: false,
            view: None,
            screen: Vec::new(),
            #[cfg(test)]
            builds: 0,
        }
    }
}

/// 同じ UV の辺は一度だけ。上限を超えたら部分的な図を出さない。
pub fn edges(triangles: &[SurfaceTriangle], material: i32, limit: usize) -> Option<Vec<[Vec2; 2]>> {
    let mut seen = HashSet::new();
    let mut edges = Vec::new();
    for t in triangles.iter().filter(|t| t.material == material) {
        for (mut a, mut b) in [(t.uv_a, t.uv_b), (t.uv_b, t.uv_c), (t.uv_c, t.uv_a)] {
            if !a.is_finite() || !b.is_finite() || a == b {
                continue;
            }
            // -0 と +0 は同じ座標として扱う。
            for p in [&mut a, &mut b] {
                if p.x == 0.0 {
                    p.x = 0.0;
                }
                if p.y == 0.0 {
                    p.y = 0.0;
                }
            }
            if b.x < a.x || (b.x == a.x && b.y < a.y) {
                std::mem::swap(&mut a, &mut b);
            }
            if seen.insert([a.x.to_bits(), a.y.to_bits(), b.x.to_bits(), b.y.to_bits()]) {
                if edges.len() == limit {
                    return None;
                }
                edges.push([a, b]);
            }
        }
    }
    Some(edges)
}

/// この `material` の辺が同じになる構造か（三角形の数・どの三角形がどのマテリアルか・この `material` の三角形の UV）。
/// 位置は見ない。ポーズで位置だけが替わった世代（`SurfaceGeometry::reposition` は UV・マテリアルが同じものだけを返す）は、
/// 辺を作り直さずに済む。UV は有限の値だけ（幾何を作るとき、有限でないものは断られる）。
fn same_edges_source(a: &SurfaceGeometry, b: &SurfaceGeometry, material: i32) -> bool {
    a.triangle_count() == b.triangle_count()
        && a.triangles().iter().zip(b.triangles()).all(|(x, y)| {
            x.material == y.material
                && (x.material != material
                    || (x.uv_a == y.uv_a && x.uv_b == y.uv_b && x.uv_c == y.uv_c))
        })
}

impl Wireframe {
    /// 表示が入のときだけ呼ぶ。辺は、UV・マテリアルの構造が替わったときだけ作る（Arc の世代が替わっただけでは作り直さない）。
    fn sync(&mut self, geometry: &Arc<SurfaceGeometry>, material: i32) {
        if let Some(held) = &self.geometry {
            let same = Arc::ptr_eq(held, geometry);
            if self.material == material && (same || same_edges_source(held, geometry, material)) {
                if !same {
                    // 次の比べを新しい世代から始める。古い世代はここで手放す。
                    self.geometry = Some(geometry.clone());
                }
                return;
            }
        }
        #[cfg(test)]
        {
            self.builds += 1;
        }
        self.geometry = Some(geometry.clone());
        self.material = material;
        let built = edges(geometry.triangles(), material, self.limit);
        self.truncated = built.is_none();
        self.edges = built.unwrap_or_default();
        self.view = None;
        self.screen.clear();
    }

    /// 表示が切れている間（またはモデル・マテリアルが無い間）。同じ幾何なら持っている辺を残し（入れ直すとすぐ出る）、
    /// 別の幾何に替わっていたら手放す。切れている間は辺を作らない。
    fn idle(&mut self, geometry: Option<&Arc<SurfaceGeometry>>) {
        let same = match (&self.geometry, geometry) {
            (Some(held), Some(now)) => Arc::ptr_eq(held, now),
            _ => false,
        };
        if !same && self.geometry.is_some() {
            self.geometry = None;
            self.material = -1;
            self.edges = Vec::new();
            self.screen = Vec::new();
            self.truncated = false;
            self.view = None;
        }
    }

    fn project(&mut self, view: &CanvasView, width: u32, height: u32) {
        if self.view.as_ref() == Some(view) {
            return;
        }
        self.screen = self
            .edges
            .iter()
            .map(|edge| {
                edge.map(|uv| {
                    view.to_screen(uv.x as f64 * width as f64, uv.y as f64 * height as f64)
                })
            })
            .collect();
        self.view = Some(*view);
    }
}

pub fn menu_entry(app: &AppState) -> crate::ui::menu::Entry<Action> {
    crate::ui::menu::Entry::item(
        app.lang.pick("UV ワイヤーフレーム", "UV Wireframe"),
        Action::ToggleUvWireframe,
    )
    .checked(app.prefs.settings.uv_wireframe)
    .enabled(app.view3d.model.is_some())
}

/// アイコンのツールチップ。名前、または今は描けない理由。
fn tip(app: &AppState) -> String {
    if app.region_model().is_none() {
        // モデルが無い・今のセットがモデルに付いていない。3D のブラシと同じ文。
        app.region_missing_reason()
    } else if app.prefs.settings.uv_wireframe && app.uv_wireframe.truncated {
        app.lang
            .pick(
                "UV の辺が表示の上限を超えています",
                "UV edges exceed the display limit",
            )
            .into()
    } else {
        app.lang.pick("UV ワイヤーフレーム", "UV Wireframe").into()
    }
}

/// キャンバスの上と、右上の入口。入力の受け皿は既存の隅の部品と同じ。
/// 重ねるのは、3D が描いている面（`region_model`＝`view3d.material`）と同じマテリアルの辺。
pub fn show(ui: &mut Ui, app: &mut AppState, view: &CanvasView) {
    let target = app.region_model();
    if let (Some((model, material)), true) = (&target, app.prefs.settings.uv_wireframe) {
        app.uv_wireframe.sync(&model.geometry, *material);
        app.uv_wireframe
            .project(view, app.doc.width(), app.doc.height());
        let c = app.prefs.settings.uv_wireframe_color;
        let stroke = Stroke::new(1.0, Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3]));
        let painter = ui.painter_at(ui.max_rect());
        painter.extend(
            app.uv_wireframe
                .screen
                .iter()
                .map(|p| egui::Shape::line_segment(*p, stroke)),
        );
    } else {
        app.uv_wireframe
            .idle(target.as_ref().map(|(model, _)| &model.geometry));
    }
    let has = app.view3d.model.is_some();
    let tip = tip(app);
    // 状態アイコンの列の下。独立した Area でキャンバスへの押下の伝播を止める。
    let mut corner = ui.max_rect();
    corner.min.y += 34.0;
    let item = w::CornerIcon::new("uv-wireframe", "uv_wireframe", tip)
        .selected(app.prefs.settings.uv_wireframe && has)
        .enabled(has);
    if w::corner_icons(ui, "uv-wireframe", corner, &[item])
        .clicked
        .is_some()
    {
        app.apply(Action::ToggleUvWireframe);
    }
}

/// 設定の既存行の末尾へ加える部品。色・不透明度のスライダーをドラッグしている間は true を返す
/// （設定のファイルへは、ドラッグの間は書かず離したときに 1 回書く）。
pub fn settings_row(ui: &mut Ui, rows: &mut w::Rows, app: &mut AppState) -> bool {
    color::settings_row(ui, rows, app)
}

pub fn parse_color(value: &str) -> Option<[u8; 4]> {
    let values: Vec<_> = value
        .split(',')
        .map(str::parse::<u8>)
        .collect::<Result<_, _>>()
        .ok()?;
    values.try_into().ok()
}

pub fn save_settings(text: &mut String, s: &crate::settings::Settings) {
    if !s.uv_wireframe {
        text.push_str("uv_wireframe=off\n");
    }
    if s.uv_wireframe_color != DEFAULT_COLOR {
        let [r, g, b, a] = s.uv_wireframe_color;
        text.push_str(&format!("uv_wireframe_color={r},{g},{b},{a}\n"));
    }
}
