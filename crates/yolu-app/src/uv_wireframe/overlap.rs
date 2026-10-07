//! 重なった UV（同じテクセルを 2 つ以上の三角形が覆う所）の図: 2D のキャンバスに重なったテクセルを色で出し、重なりに関わる島の縁を
//! 太く描く（表示のメニューの「重なった UV」、色は UV ワイヤーフレームの色の並び）。塗りの知らせ（`note_*`）も同じ図を読む。
//!
//! 図はベイクと同じ行の割り当て（core の `uv_overlap`。1 テクセルに中心の 1 点）で、受けたままのモデルの形・今のセットのマテリアル・
//! 文書の大きさごとに 1 回、別のスレッドで数える（このマテリアルの UV と三角形の並びが同じなら、ポーズで位置だけ替わっても数え直さない）。
//! 縁を描く島はベイクの優先と同じ島（UV と位置の両方でつながる三角形）なので、ミラーで UV がぴったり重なった両側も、それぞれの島の縁になる。

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use egui::{Color32, ColorImage, Painter, Pos2, Shape, Stroke, TextureHandle, TextureOptions};
use yolu_core::geometry::SurfaceGeometry;
use yolu_core::glam::Vec2;
use yolu_core::mesh_maps::{islands_of, uv_overlap, UvOverlap};

use crate::canvas::display::{CanvasDisplay, NEAREST_FROM_PIXEL_SIZE, PAGE};
use crate::canvas::view::CanvasView;
use crate::jobs::{JobSpec, Polled, Worker};
use crate::notice::Source;
use crate::state::{Action, AppState};

/// 既定の色（警告の色 `t::WARNING` に近い橙。テクセルを塗る不透明度）。
pub const DEFAULT_COLOR: [u8; 4] = [232, 176, 60, 120];
/// 島の縁の線の上限（これを超える図は縁を出さない。テクセルの色は出す）。
const MAX_EDGES: usize = 1 << 20;

/// 数えている間は描き直す（終わりを受ける）。
pub(crate) const JOB: JobSpec = JobSpec {
    repaint: true,
    ..JobSpec::new("uv.overlap", |app| app.uv_overlap.is_counting())
};

/// 数えた図と、重なりに関わる島の縁（UV の線）。
pub struct Built {
    pub map: UvOverlap,
    pub edges: Vec<[Vec2; 2]>,
}

/// 重なりの色のテクスチャと、それが覆う図のテクセルの幅・高さ（まとめた分だけ図より大きいことがある）。
struct Page {
    width: u32,
    height: u32,
    texture: TextureHandle,
}

/// 図の状態（モデル・マテリアル・大きさごと）。
#[derive(Default)]
pub struct OverlapState {
    /// 数えた（数えている）元: 幾何・マテリアル・文書の大きさ。
    key: Option<(Arc<SurfaceGeometry>, i32, u32, u32)>,
    worker: Option<Worker<Option<Result<Built, String>>>>,
    built: Option<Arc<Built>>,
    page: Option<Page>,
    /// テクスチャを作った図・色・補間。
    pages_for: Option<(usize, [u8; 4], bool)>,
    screen: Vec<[Pos2; 2]>,
    screen_for: Option<(CanvasView, usize)>,
    /// 塗りの知らせを出したテクスチャセット（uid。起動の間）。
    noticed: HashSet<u32>,
    /// 数えた回数（試験用）。
    #[doc(hidden)]
    pub counts: u32,
}

impl OverlapState {
    pub fn is_counting(&self) -> bool {
        self.worker.is_some()
    }

    /// 数えた図（まだ・モデルが無ければ None）。
    pub fn built(&self) -> Option<&Arc<Built>> {
        self.built.as_ref()
    }

    fn clear(&mut self) {
        self.key = None;
        self.worker = None;
        self.built = None;
        self.page = None;
        self.pages_for = None;
        self.screen.clear();
        self.screen_for = None;
    }

    fn start(&mut self, geometry: Arc<SurfaceGeometry>, material: i32, width: u32, height: u32) {
        self.clear();
        self.key = Some((geometry.clone(), material, width, height));
        self.counts += 1;
        let worker = Worker::spawn("yolu-uv-overlap", move |tx, cancel| {
            let _ = tx.send(count(&geometry, material, width, height, cancel.flag()));
        });
        match worker {
            Ok(w) => self.worker = Some(w.cancel_on_drop()),
            // スレッドを作れなければ図を出さない（知らせもしない。表示の補助なので）
            Err(_) => self.worker = None,
        }
    }
}

/// 別のスレッドで数える。取り消されたら None。
fn count(
    geometry: &SurfaceGeometry,
    material: i32,
    width: u32,
    height: u32,
    cancel: &std::sync::atomic::AtomicBool,
) -> Option<Result<Built, String>> {
    let tris = geometry.triangles();
    let n = tris.len();
    let mut uvs = Vec::with_capacity(n * 6);
    let mut corners = Vec::with_capacity(n * 9);
    let mut slots = Vec::with_capacity(n);
    for t in tris {
        uvs.extend_from_slice(&[t.uv_a.x, t.uv_a.y, t.uv_b.x, t.uv_b.y, t.uv_c.x, t.uv_c.y]);
        for p in [t.a, t.b, t.c] {
            corners.extend_from_slice(&[p.x, p.y, p.z]);
        }
        slots.push(t.material_slot);
    }
    let mine: Vec<usize> = (0..n).filter(|&i| tris[i].material == material).collect();
    let map = match uv_overlap(&uvs, &mine, width, height, Some(cancel)) {
        Ok(Some(m)) => m,
        Ok(None) => return None,
        Err(e) => return Some(Err(e.to_string())),
    };
    let edges = if map.is_empty() {
        Vec::new()
    } else {
        let islands = islands_of(&corners, &uvs, &slots);
        let involved: HashSet<usize> = map
            .triangles()
            .iter()
            .map(|t| islands[*t as usize])
            .collect();
        island_edges(geometry, &islands, &involved, MAX_EDGES)
            .map(|edges| edges.into_iter().map(|(_, e)| e).collect())
            .unwrap_or_default()
    };
    Some(Ok(Built { map, edges }))
}

/// 島ごとの縁（島の中で 1 つの三角形にしか属さない UV の辺。島の番号と線）。島ごとに数えるので、UV がぴったり重なった 2 つの島も、
/// それぞれの縁になる。`involved` の島だけ。線が `limit` を超えたら None。並びは島の番号の昇順、同じ島の中は座標の順（同じ図は同じ線の列）。
pub(crate) fn island_edges(
    geometry: &SurfaceGeometry,
    islands: &[usize],
    involved: &HashSet<usize>,
    limit: usize,
) -> Option<Vec<(usize, [Vec2; 2])>> {
    type Point = (i64, i64);
    let q = |v: Vec2| -> Point {
        (
            (v.x as f64 * 1e6).round_ties_even() as i64,
            (v.y as f64 * 1e6).round_ties_even() as i64,
        )
    };
    let mut count: HashMap<(usize, Point, Point), (u32, [Vec2; 2])> = HashMap::new();
    for (i, t) in geometry.triangles().iter().enumerate() {
        let Some(&island) = islands.get(i) else {
            break;
        };
        if !involved.contains(&island) {
            continue;
        }
        for (a, b) in [(t.uv_a, t.uv_b), (t.uv_b, t.uv_c), (t.uv_c, t.uv_a)] {
            let key = if q(a) <= q(b) {
                (island, q(a), q(b))
            } else {
                (island, q(b), q(a))
            };
            count.entry(key).or_insert((0, [a, b])).0 += 1;
        }
    }
    let mut out: Vec<(usize, [Vec2; 2])> = count
        .into_iter()
        .filter(|(_, (n, _))| *n == 1)
        .map(|((island, _, _), (_, e))| (island, e))
        .collect();
    if out.len() > limit {
        return None;
    }
    out.sort_by_key(|(island, e)| {
        (
            *island,
            e[0].x.to_bits(),
            e[0].y.to_bits(),
            e[1].x.to_bits(),
            e[1].y.to_bits(),
        )
    });
    Some(out)
}

impl AppState {
    /// 今のモデル（受けたままの形）・セットのマテリアル・文書の大きさで図を数え直す（元が替わったときだけ）と、数え終わりを受ける。
    /// 毎フレーム呼ぶ。
    pub fn poll_uv_overlap(&mut self) {
        let material = self.view3d.material;
        let target = self
            .view3d
            .full_model()
            .map(|m| m.geometry.clone())
            .filter(|_| material >= 0);
        let (width, height) = (self.doc.width(), self.doc.height());
        let s = &mut self.uv_overlap;
        let Some(geometry) = target else {
            s.clear();
            return;
        };
        let same = s.key.as_ref().is_some_and(|(held, m, w, h)| {
            *m == material
                && (*w, *h) == (width, height)
                && (Arc::ptr_eq(held, &geometry)
                    || super::same_edges_source(held, &geometry, material))
        });
        if !same {
            s.start(geometry, material, width, height);
        } else if let Some(key) = s.key.as_mut() {
            // 位置だけ替わった世代へ持ち替える（古い世代を握り続けない）
            if !Arc::ptr_eq(&key.0, &geometry) {
                key.0 = geometry;
            }
        }
        let Some(worker) = s.worker.as_ref() else {
            return;
        };
        match worker.poll() {
            Polled::Empty => {}
            Polled::Message(Some(Ok(built))) => {
                s.worker = None;
                s.built = Some(Arc::new(built));
                s.pages_for = None;
                s.screen_for = None;
            }
            // 取り消し・数えられない（UV の行帯が予算を超えるなど）・スレッドの異常: 図を出さない
            Polled::Message(None) | Polled::Message(Some(Err(_))) | Polled::Lost => {
                s.worker = None;
            }
        }
    }

    /// 塗りの知らせ: 2D の点（画素の座標、左下原点）と半径（画素）の中に重なったテクセルがあれば、テクスチャセットごとに 1 度だけ知らせる。
    pub fn note_overlap_canvas(&mut self, x: f64, y: f64, radius: f64) {
        let uid = self.sets.current().uid;
        let Some(built) = self.overlap_for_notice(uid) else {
            return;
        };
        let r = radius.max(0.5);
        if built.map.any_in(
            (x - r).floor() as i64,
            (y - r).floor() as i64,
            (x + r).floor() as i64,
            (y + r).floor() as i64,
        ) {
            self.notice_overlap(uid);
        }
    }

    /// 塗りの知らせ: 3D で描いた面の UV（0〜1）の点のテクセルが重なっていれば、テクスチャセットごとに 1 度だけ知らせる。
    pub fn note_overlap_uv(&mut self, uv: Vec2) {
        let uid = self.sets.current().uid;
        let Some(built) = self.overlap_for_notice(uid) else {
            return;
        };
        let (w, h) = (built.map.width() as f64, built.map.height() as f64);
        let (x, y) = (uv.x as f64 * w, uv.y as f64 * h);
        if built.map.any_in(
            x.floor() as i64,
            y.floor() as i64,
            x.floor() as i64,
            y.floor() as i64,
        ) {
            self.notice_overlap(uid);
        }
    }

    /// まだ知らせていないセットで、今の文書の大きさの図があれば、その図。
    fn overlap_for_notice(&self, uid: u32) -> Option<Arc<Built>> {
        let s = &self.uv_overlap;
        if s.noticed.contains(&uid) {
            return None;
        }
        s.built
            .clone()
            .filter(|b| !b.map.is_empty())
            .filter(|b| (b.map.width(), b.map.height()) == (self.doc.width(), self.doc.height()))
    }

    fn notice_overlap(&mut self, uid: u32) {
        self.uv_overlap.noticed.insert(uid);
        self.info(
            Source::Brush,
            self.lang.pick(
                "重なった UV は同じテクセルを使うので、片側だけには描けません。",
                "Overlapping UVs share the same texels, so one side cannot be painted alone.",
            ),
        );
    }
}

/// 塗りの知らせ（3D）: 描いた点の下の今のセットの面の UV で `note_overlap_uv`。もう知らせたセット・図が無いときは、面を引かない。
pub fn note_surface(app: &mut AppState, rect: egui::Rect, at: Pos2) {
    let uid = app.sets.current().uid;
    if app.overlap_for_notice(uid).is_none() {
        return;
    }
    let Some(model) = app.view3d.model.clone() else {
        return;
    };
    let view = app.view3d.camera.view(rect.width(), rect.height());
    let p = Vec2::new(at.x - rect.left(), at.y - rect.top());
    if let Some(hit) = yolu_core::geometry::pick(&model.geometry, &view, p) {
        if hit.material == app.view3d.material {
            app.note_overlap_uv(hit.uv);
        }
    }
}

/// 表示のメニューの入口（UV ワイヤーフレームの下）。
pub fn menu_entry(app: &AppState) -> crate::ui::menu::Entry<Action> {
    crate::ui::menu::Entry::item(
        app.lang.pick("重なった UV", "Overlapping UVs"),
        Action::ToggleUvOverlap,
    )
    .checked(app.prefs.settings.uv_overlap)
    .enabled(app.view3d.model.is_some())
}

/// 2D のキャンバスに描く（入のとき）: 重なったテクセルを色で、重なりに関わる島の縁を太い線で。
pub fn paint(painter: &Painter, app: &mut AppState, view: &CanvasView) {
    if !app.prefs.settings.uv_overlap {
        return;
    }
    let Some(built) = app.uv_overlap.built.clone() else {
        return;
    };
    if built.map.is_empty() {
        return;
    }
    let color = app.prefs.settings.uv_overlap_color;
    let (doc_w, doc_h) = (app.doc.width(), app.doc.height());
    let nearest = view.pixel_size() >= NEAREST_FROM_PIXEL_SIZE;
    let id = Arc::as_ptr(&built) as usize;
    let s = &mut app.uv_overlap;
    if s.pages_for != Some((id, color, nearest)) {
        s.page = texture(painter.ctx(), &built.map, color, nearest);
        s.pages_for = Some((id, color, nearest));
    }
    let (mw, mh) = (built.map.width() as f64, built.map.height() as f64);
    let (dw, dh) = (doc_w as f64, doc_h as f64);
    if let Some(page) = &s.page {
        let x1 = page.width as f64 / mw * dw;
        let y1 = page.height as f64 / mh * dh;
        CanvasDisplay::quad(
            painter,
            page.texture.id(),
            view,
            [(0.0, 0.0), (x1, 0.0), (x1, y1), (0.0, y1)],
            [
                egui::pos2(0.0, 0.0),
                egui::pos2(1.0, 0.0),
                egui::pos2(1.0, 1.0),
                egui::pos2(0.0, 1.0),
            ],
        );
    }
    if s.screen_for != Some((*view, id)) {
        s.screen = built
            .edges
            .iter()
            .map(|e| e.map(|uv| view.to_screen(uv.x as f64 * dw, uv.y as f64 * dh)))
            .collect();
        s.screen_for = Some((*view, id));
    }
    let line = Stroke::new(
        2.0,
        Color32::from_rgba_unmultiplied(color[0], color[1], color[2], color[3].max(200)),
    );
    painter.extend(s.screen.iter().map(|p| Shape::line_segment(*p, line)));
}

/// ベイクの窓の UV の見取り図に重ねる、重なりの色のテクスチャ（キャンバスと同じもの。設定の色）と、それが覆う UV の幅・高さ
/// （まとめた分だけ 1 を超えることがある）。図がまだ・重なりが無い・今の文書の大きさの図でなければ None。キャンバスが作った
/// テクスチャがあれば補間によらずそれを使い、無ければ線形の補間で作る。
pub fn texture_for_map(
    ctx: &egui::Context,
    app: &mut AppState,
) -> Option<(egui::TextureId, [f32; 2])> {
    let built = app.uv_overlap.built.clone()?;
    if built.map.is_empty()
        || (built.map.width(), built.map.height()) != (app.doc.width(), app.doc.height())
    {
        return None;
    }
    let color = app.prefs.settings.uv_overlap_color;
    let id = Arc::as_ptr(&built) as usize;
    let s = &mut app.uv_overlap;
    if !s
        .pages_for
        .is_some_and(|(held, c, _)| held == id && c == color)
    {
        s.page = texture(ctx, &built.map, color, false);
        s.pages_for = Some((id, color, false));
    }
    let page = s.page.as_ref()?;
    Some((
        page.texture.id(),
        [
            page.width as f32 / built.map.width() as f32,
            page.height as f32 / built.map.height() as f32,
        ],
    ))
}

/// 重なったテクセルを色のテクスチャにする（行の区間から。左下原点の行を下から並べる）。図の大きさが `PAGE` を超えるときは、`PAGE` に
/// 収まるまで k × k のテクセルを 1 画素にまとめる（どれか 1 つでも重なっていれば色。表示の補助なので、大きな文書でも GPU のテクスチャは
/// 1 枚・16 MiB まで）。重なりが無ければ None。
fn texture(ctx: &egui::Context, map: &UvOverlap, color: [u8; 4], nearest: bool) -> Option<Page> {
    if map.is_empty() {
        return None;
    }
    let options = TextureOptions {
        magnification: if nearest {
            egui::TextureFilter::Nearest
        } else {
            egui::TextureFilter::Linear
        },
        minification: egui::TextureFilter::Linear,
        wrap_mode: egui::TextureWrapMode::ClampToEdge,
        mipmap_mode: None,
    };
    let fill = Color32::from_rgba_unmultiplied(color[0], color[1], color[2], color[3]);
    let (w, h, step, marks) = mask(map, PAGE);
    let pixels = marks
        .into_iter()
        .map(|m| if m { fill } else { Color32::TRANSPARENT })
        .collect();
    let texture = ctx.load_texture(
        "uv-overlap",
        ColorImage::new([w as usize, h as usize], pixels),
        options,
    );
    Some(Page {
        width: w * step,
        height: h * step,
        texture,
    })
}

/// 重なったテクセルの印を `limit` × `limit` に収まるようにまとめた面（幅・高さ・まとめた辺 k・印。左下原点の行を下から）。
fn mask(map: &UvOverlap, limit: u32) -> (u32, u32, u32, Vec<bool>) {
    let (width, height) = (map.width(), map.height());
    let step = width.max(height).div_ceil(limit.max(1)).max(1);
    let (w, h) = (width.div_ceil(step), height.div_ceil(step));
    let mut marks = vec![false; (w * h) as usize];
    for y in 0..height {
        let row = (y / step * w) as usize;
        for r in map.row(y) {
            let (a, b) = (r[0] / step, (r[1] - 1) / step);
            marks[row + a as usize..=row + b as usize].fill(true);
        }
    }
    (w, h, step, marks)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_large_map_is_shown_on_one_texture_keeping_every_overlapped_texel() {
        // 2 つの三角形が UV の (0.25〜0.75) で重なる 16 × 16 の図を、4 × 4 に収める
        let uv = [0.25f32, 0.25, 0.75, 0.25, 0.25, 0.75];
        let uvs = [uv, uv].concat();
        let map = uv_overlap(&uvs, &[0, 1], 16, 16, None).unwrap().unwrap();
        assert!(map.texel_count() > 0);
        let (w, h, step, marks) = mask(&map, 4);
        assert_eq!((w, h, step), (4, 4, 4));
        for y in 0..16 {
            for x in 0..16 {
                if map.contains(x, y) {
                    assert!(marks[((y / 4) * 4 + x / 4) as usize], "({x}, {y})");
                }
            }
        }
        // 重なりの無いまとまりは印を付けない（左下の角）
        assert!(!marks[0]);
        // 収まる大きさならまとめない
        let (w, h, step, marks) = mask(&map, 16);
        assert_eq!((w, h, step), (16, 16, 1));
        assert_eq!(
            marks.iter().filter(|m| **m).count() as u64,
            map.texel_count()
        );
    }
}
