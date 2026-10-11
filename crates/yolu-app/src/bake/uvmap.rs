//! ベイクのウィンドウの「重なった UV」の UV の見取り図: 今のセットの UV の配置（受けたままのモデルの形）を正方形に描き、アイランドの上で押すと
//! そのアイランドのメニュー（優先する・焼かない・外す）を出す。アイランドはベイクと同じ `bake_islands`（ミラーで UV がぴったり重なった両側は別のアイランド）。
//!
//! アイランドは薄く塗り、重なったテクセルはキャンバスと同じ重なりの色のテクスチャ（設定の色）、優先するアイランドはアクセントの色の縁、焼かないアイランドは
//! 薄い塗りに斜線。ポインタを置いたアイランド（メニューを開いているアイランド）は白い縁で強調し、キャンバスと 3D ビューでも同じアイランドを強調する
//! （ミラーの両側は UV では同じ所に見えるので、どちらの側かは 3D で見分ける）。重なった所を続けて押すと、重なったアイランドを順に選ぶ。
//! ホイールで拡大、中ボタンか Space ＋ 左ドラッグで動かす（キャンバスと同じ）。0〜1 の外は見せない（拡大は 1 から）。

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::sync::Arc;

use egui::epaint::{Mesh, Vertex, WHITE_UV};
use egui::{
    pos2, Color32, ColorImage, CursorIcon, Event, Id, PointerButton, Pos2, Rect, Sense, Shape,
    Stroke, TextureHandle, TextureId, TextureOptions, Ui,
};
use yolu_core::geometry::SurfaceGeometry;
use yolu_core::glam::Vec2;
use yolu_core::mesh_maps::MeshOverlapPriority;

use super::overlap::Islands;
use crate::region::index::UvGrid;
use crate::state::AppState;
use crate::ui::theme as t;

/// アイランドの薄い塗り・焼かないアイランドの塗りと斜線・縁の色。
const FILL: Color32 = Color32::from_rgba_unmultiplied_const(217, 217, 220, 40);
const SKIP_FILL: Color32 = Color32::from_rgba_unmultiplied_const(217, 217, 220, 10);
const SKIP_HATCH: Color32 = Color32::from_rgba_unmultiplied_const(154, 154, 160, 120);
const EDGE: Color32 = Color32::from_rgba_unmultiplied_const(217, 217, 220, 70);
const SKIP_EDGE: Color32 = Color32::from_rgba_unmultiplied_const(154, 154, 160, 110);
const HOVER_FILL: Color32 = Color32::from_rgba_unmultiplied_const(255, 255, 255, 46);
/// 1 つのメッシュの三角形の数。
const CHUNK: usize = 20_000;
/// 全部のアイランドの縁の線の上限（これを超える図は、優先・焼かない・強調のアイランドの縁だけを出す）。
const MAX_EDGES: usize = 1 << 18;
/// 拡大の上限。
const MAX_ZOOM: f32 = 64.0;
/// 同じ所を続けて押したとみなす距離（px。範囲のツールの選び替えと同じ）。
const CYCLE_RADIUS: f32 = 4.0;
/// 斜線のテクスチャの 1 辺（画面の点）。
const HATCH: usize = 8;
/// 拡大 1 のときの、0〜1 の正方形のまわりの余白（画面の点）。
const PAD: f32 = 10.0;
/// 0〜1 の正方形の地の色。
const SQUARE_BG: Color32 = Color32::from_rgb(0x1B, 0x1B, 0x1E);

/// 見取り図の中身（モデル・マテリアル・アイランドごと。ウィンドウが出ている間に 1 回作る）。
pub struct MapData {
    geometry: Arc<SurfaceGeometry>,
    material: i32,
    islands: Arc<Islands>,
    grid: UvGrid,
    /// このセットの三角形（受けたままの形の番号の昇順）。
    tris: Vec<u32>,
    /// アイランドごとの縁（アイランドの番号の昇順）と、アイランドごとの範囲。
    edges: Vec<[Vec2; 2]>,
    ranges: HashMap<usize, Range<usize>>,
    /// 全部のアイランドの縁を出すか（線が上限を超えたら出さない）。
    all_edges: bool,
}

impl MapData {
    fn new(geometry: Arc<SurfaceGeometry>, material: i32, islands: Arc<Islands>) -> MapData {
        let triangles = geometry.triangles();
        let tris: Vec<u32> = (0..triangles.len() as u32)
            .filter(|&i| triangles[i as usize].material == material)
            .collect();
        let involved: HashSet<usize> = tris
            .iter()
            .filter_map(|&i| islands.island(i as usize))
            .collect();
        let grouped = crate::uv_wireframe::overlap::island_edges(
            &geometry,
            islands.of(),
            &involved,
            usize::MAX,
        )
        .unwrap_or_default();
        let all_edges = grouped.len() <= MAX_EDGES;
        let mut ranges: HashMap<usize, Range<usize>> = HashMap::new();
        let mut edges = Vec::with_capacity(grouped.len());
        for (i, (island, e)) in grouped.into_iter().enumerate() {
            ranges.entry(island).or_insert(i..i).end = i + 1;
            edges.push(e);
        }
        let grid = UvGrid::new(&geometry, material);
        MapData {
            geometry,
            material,
            islands,
            grid,
            tris,
            edges,
            ranges,
            all_edges,
        }
    }

    /// このアイランドの索引の上に作った見取り図か（見取り図は索引を握る）。
    pub(super) fn is_on(&self, islands: &Arc<Islands>) -> bool {
        Arc::ptr_eq(&self.islands, islands)
    }

    fn is_for(
        &self,
        geometry: &Arc<SurfaceGeometry>,
        material: i32,
        islands: &Arc<Islands>,
    ) -> bool {
        self.material == material
            && Arc::ptr_eq(&self.geometry, geometry)
            && Arc::ptr_eq(&self.islands, islands)
    }

    /// UV の点の下のアイランド（代表の三角形。点の下の一番小さい三角形の番号の順、同じアイランドは 1 つ）。0〜1 の外は空。
    pub fn islands_at(&self, uv: Vec2) -> Vec<usize> {
        let mut out: Vec<usize> = Vec::new();
        for t in self.grid.find_all(uv) {
            if let Some(r) = self.islands.representative(t as usize) {
                if !out.contains(&r) {
                    out.push(r);
                }
            }
        }
        out
    }

    /// アイランド（代表）の縁。
    fn edges_of(&self, representative: usize) -> &[[Vec2; 2]] {
        self.islands
            .island(representative)
            .and_then(|i| self.ranges.get(&i))
            .map_or(&[], |r| &self.edges[r.clone()])
    }
}

impl AppState {
    /// ベイクのウィンドウの見取り図の中身（今のモデル・セット。モデルの入力を別のスレッドで作っている間・モデルが無いときは None）。
    pub fn overlap_map(&mut self) -> Option<Arc<MapData>> {
        let material = self.view3d.material;
        let geometry = self
            .view3d
            .full_model()
            .map(|m| m.geometry.clone())
            .filter(|_| material >= 0)?;
        let islands = self.overlap_islands(false)?.ok()?;
        if islands.of().len() != geometry.triangle_count() {
            return None;
        }
        if let Some(m) = self
            .bake
            .map
            .as_ref()
            .filter(|m| m.is_for(&geometry, material, &islands))
        {
            return Some(m.clone());
        }
        let map = Arc::new(MapData::new(geometry, material, islands));
        self.bake.map = Some(map.clone());
        Some(map)
    }
}

/// 見取り図の 1 フレームに描くもの（描く前に集める）。
pub struct MapFrame {
    pub data: Option<Arc<MapData>>,
    pub priority: MeshOverlapPriority,
    /// 重なりの色のテクスチャと、それが覆う UV の幅・高さ。
    pub overlap: Option<(TextureId, [f32; 2])>,
    /// 見取り図から開いたメニューのアイランド（代表）。
    pub menu: Option<usize>,
}

/// 見取り図の操作の結果。
#[derive(Default)]
pub struct MapOutcome {
    /// ポインタを置いているアイランド（代表）。
    pub hover: Option<usize>,
    /// 押したアイランド（代表）と、メニューを開く点。
    pub menu: Option<(usize, Pos2)>,
}

/// 続けて押した所の選び替え。
struct Cycle {
    at: Pos2,
    keys: Vec<usize>,
    index: usize,
}

/// 描いた形の控え（同じなら作り直さない）。
struct Drawn {
    data: usize,
    priority: MeshOverlapPriority,
    rect: Rect,
    zoom: f32,
    center: Vec2,
    under: Vec<Shape>,
    over: Vec<Shape>,
}

/// 見取り図の画面の状態（拡大・中心・選び替え・描いた形）。
pub struct MapUi {
    /// 拡大（1 で 0〜1 の正方形が見取り図いっぱい）。
    zoom: f32,
    /// 見取り図の中心の UV。
    center: Vec2,
    /// 最後に描いた見取り図の矩形（ホイールを右の欄のスクロールに渡さない判定）。
    pub rect: Rect,
    cycle: Option<Cycle>,
    /// 左ボタンを押した所（離したときに近ければ押下）。
    press: Option<Pos2>,
    /// 前のフレームに、見取り図から開いたアイランドのメニューがあった。
    menu_open: bool,
    drawn: Option<Drawn>,
    hatch: Option<TextureHandle>,
}

impl Default for MapUi {
    fn default() -> Self {
        MapUi {
            zoom: 1.0,
            center: Vec2::splat(0.5),
            rect: Rect::NOTHING,
            cycle: None,
            press: None,
            menu_open: false,
            drawn: None,
            hatch: None,
        }
    }
}

/// UV と画面の写し。
#[derive(Clone, Copy)]
struct Frame {
    rect: Rect,
    scale: f32,
    center: Vec2,
}

impl Frame {
    fn to_screen(self, uv: Vec2) -> Pos2 {
        let c = self.rect.center();
        pos2(
            c.x + (uv.x - self.center.x) * self.scale,
            c.y - (uv.y - self.center.y) * self.scale,
        )
    }
    fn to_uv(self, p: Pos2) -> Vec2 {
        let c = self.rect.center();
        Vec2::new(
            self.center.x + (p.x - c.x) / self.scale,
            self.center.y - (p.y - c.y) / self.scale,
        )
    }
}

impl MapUi {
    /// 拡大（1 で 0〜1 の正方形が見取り図いっぱい）。
    pub fn zoom(&self) -> f32 {
        self.zoom
    }

    /// 拡大は 1〜`MAX_ZOOM`、中心は 0〜1 の正方形が見取り図を覆う範囲に収める。
    fn settle(&mut self) {
        self.zoom = if self.zoom.is_finite() {
            self.zoom.clamp(1.0, MAX_ZOOM)
        } else {
            1.0
        };
        let half = 0.5 / self.zoom;
        let fix = |v: f32| {
            if v.is_finite() {
                v.clamp(half, 1.0 - half)
            } else {
                0.5
            }
        };
        self.center = Vec2::new(fix(self.center.x), fix(self.center.y));
    }

    fn frame(&self, rect: Rect) -> Frame {
        Frame {
            rect,
            scale: (rect.width() - 2.0 * PAD).max(1.0) * self.zoom,
            center: self.center,
        }
    }

    /// 点の下の候補（`keys`）のうち選んでいるアイランド: 続けて同じ所なら前の候補（`press` なら次へ進めて覚える）、そうでなければ先頭。
    fn choose(&mut self, at: Pos2, keys: Vec<usize>, press: bool) -> Option<usize> {
        if keys.is_empty() {
            if press {
                self.cycle = None;
            }
            return None;
        }
        let previous = self
            .cycle
            .as_ref()
            .filter(|c| c.at.distance(at) <= CYCLE_RADIUS && c.keys == keys)
            .map(|c| c.index);
        if !press {
            return keys.get(previous.unwrap_or(0)).copied();
        }
        let index = previous.map_or(0, |i| (i + 1) % keys.len());
        let island = keys[index];
        self.cycle = Some(Cycle { at, keys, index });
        Some(island)
    }

    fn hatch(&mut self, ctx: &egui::Context) -> TextureId {
        self.hatch
            .get_or_insert_with(|| {
                // 2 点の太さの斜線（左下から右上）
                let pixels = (0..HATCH * HATCH)
                    .map(|i| {
                        let (x, y) = (i % HATCH, i / HATCH);
                        if (x + y) % HATCH < 2 {
                            Color32::WHITE
                        } else {
                            Color32::TRANSPARENT
                        }
                    })
                    .collect();
                ctx.load_texture(
                    "bake-map-hatch",
                    ColorImage::new([HATCH, HATCH], pixels),
                    TextureOptions {
                        magnification: egui::TextureFilter::Nearest,
                        minification: egui::TextureFilter::Nearest,
                        wrap_mode: egui::TextureWrapMode::Repeat,
                        mipmap_mode: None,
                    },
                )
            })
            .id()
    }
}

/// アイランド（代表）の三角形を 1 色で塗るメッシュ（`texture` があれば画面の点の位置で斜線を貼る）。
fn triangles_mesh(
    data: &MapData,
    tris: &[u32],
    frame: &Frame,
    color: Color32,
    texture: Option<TextureId>,
) -> Vec<Shape> {
    let triangles = data.geometry.triangles();
    let mut out = Vec::new();
    for chunk in tris.chunks(CHUNK) {
        let mut mesh = match texture {
            Some(id) => Mesh::with_texture(id),
            None => Mesh::default(),
        };
        for &i in chunk {
            let t = &triangles[i as usize];
            let n = mesh.vertices.len() as u32;
            for uv in [t.uv_a, t.uv_b, t.uv_c] {
                let pos = frame.to_screen(uv);
                let tex = match texture {
                    Some(_) => pos2(pos.x / HATCH as f32, pos.y / HATCH as f32),
                    None => WHITE_UV,
                };
                mesh.vertices.push(Vertex {
                    pos,
                    uv: tex,
                    color,
                });
            }
            mesh.indices.extend_from_slice(&[n, n + 1, n + 2]);
        }
        if !mesh.is_empty() {
            out.push(Shape::mesh(mesh));
        }
    }
    out
}

fn segments<'a>(
    edges: &'a [[Vec2; 2]],
    frame: &Frame,
    stroke: Stroke,
) -> impl Iterator<Item = Shape> + 'a {
    let f = *frame;
    edges
        .iter()
        .map(move |e| Shape::line_segment([f.to_screen(e[0]), f.to_screen(e[1])], stroke))
}

/// アイランドの塗り（重なりの色の下）と、焼かないアイランドの斜線・縁（重なりの色の上）。優先・焼かないの状態で決まる（ポインタの強調は入れない）。
fn base_shapes(
    ui: &mut MapUi,
    ctx: &egui::Context,
    data: &MapData,
    priority: &MeshOverlapPriority,
    frame: &Frame,
) -> (Vec<Shape>, Vec<Shape>) {
    // 別のモデルの一覧は、ここでは使わない（ウィンドウに注意の行が出る）
    let own = priority.binding() == data.islands.binding();
    let island_set = |set: &std::collections::BTreeSet<usize>| -> HashSet<usize> {
        if !own {
            return HashSet::new();
        }
        set.iter().filter_map(|t| data.islands.island(*t)).collect()
    };
    let skipped = island_set(priority.skipped());
    let (skip_tris, normal_tris): (Vec<u32>, Vec<u32>) = data.tris.iter().partition(|&&t| {
        data.islands
            .island(t as usize)
            .is_some_and(|i| skipped.contains(&i))
    });
    let hatch = ui.hatch(ctx);
    let mut under = triangles_mesh(data, &normal_tris, frame, FILL, None);
    under.extend(triangles_mesh(data, &skip_tris, frame, SKIP_FILL, None));
    let mut shapes = Vec::new();
    shapes.extend(triangles_mesh(
        data,
        &skip_tris,
        frame,
        SKIP_HATCH,
        Some(hatch),
    ));
    if data.all_edges {
        shapes.extend(segments(&data.edges, frame, Stroke::new(1.0, EDGE)));
    }
    if own {
        for &t in priority.skipped() {
            shapes.extend(segments(
                data.edges_of(t),
                frame,
                Stroke::new(1.0, SKIP_EDGE),
            ));
        }
        for &t in priority.preferred() {
            shapes.extend(segments(
                data.edges_of(t),
                frame,
                Stroke::new(2.0, t::ACCENT),
            ));
        }
    }
    (under, shapes)
}

/// 見取り図を描いて、押下・ホイール・ドラッグを受ける。`list_hover` は一覧の行で指しているアイランド（代表）。
pub fn draw(
    ui: &mut Ui,
    rect: Rect,
    state: &mut MapUi,
    map: &MapFrame,
    list_hover: Option<usize>,
) -> MapOutcome {
    let mut out = MapOutcome::default();
    state.rect = rect;
    let ctx = ui.ctx().clone();
    let response = ui.interact(rect, Id::new("bake.overlap.map"), Sense::click_and_drag());
    let pointer = response.hover_pos();
    // ホイールで拡大（ポインタの下の UV は動かない。1 目盛りで約 1.23 倍、キャンバスと同じ）
    if let Some(p) = pointer {
        let notches: f32 = ui.input(|i| {
            i.events
                .iter()
                .filter_map(|e| match e {
                    Event::MouseWheel { unit, delta, .. } => Some(match unit {
                        egui::MouseWheelUnit::Point => delta.y / 40.0,
                        egui::MouseWheelUnit::Line => delta.y,
                        egui::MouseWheelUnit::Page => delta.y * 3.0,
                    }),
                    _ => None,
                })
                .sum()
        });
        if notches != 0.0 {
            let before = state.frame(rect).to_uv(p);
            state.zoom *= (notches * 0.21).exp();
            state.settle();
            let after = state.frame(rect).to_uv(p);
            state.center += before - after;
            state.settle();
        }
    }
    // 中ボタンか Space ＋ 左ドラッグで動かす
    let space = ui.input(|i| crate::keymap::hold_down(i, "view.pan_hold"))
        && !ctx.egui_wants_keyboard_input();
    let panning = response.dragged_by(PointerButton::Middle)
        || (space && response.dragged_by(PointerButton::Primary));
    if panning {
        let d = response.drag_delta();
        let scale = state.frame(rect).scale;
        state.center += Vec2::new(-d.x / scale, d.y / scale);
        state.settle();
        ctx.set_cursor_icon(CursorIcon::Grabbing);
    } else if space && response.hovered() {
        ctx.set_cursor_icon(CursorIcon::Grab);
    }
    state.settle();
    let frame = state.frame(rect);

    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    painter.rect_filled(rect, 0.0, t::CANVAS_BG);
    let square = Rect::from_two_pos(
        frame.to_screen(Vec2::new(0.0, 0.0)),
        frame.to_screen(Vec2::new(1.0, 1.0)),
    );
    painter.rect_filled(square, 0.0, SQUARE_BG);
    painter.rect_stroke(
        square,
        0.0,
        Stroke::new(1.0, t::TEXT_DISABLED),
        egui::StrokeKind::Outside,
    );
    if let Some(data) = &map.data {
        // 点の下のアイランド（押せば続けて押した所の次のアイランド）
        let under = |p: Pos2| data.islands_at(frame.to_uv(p));
        // 押下は生の入力で見る: 見取り図から開いたメニューの受け皿が上にあっても、メニューの外の見取り図を押せば、そのまま
        // 次のアイランドを選べる（受け皿の押しはメニューを閉じるだけで、見取り図の部品には届かない）
        let (pressed, released, origin, latest) = ui.input(|i| {
            (
                i.pointer.primary_pressed(),
                i.pointer.primary_released(),
                i.pointer.press_origin(),
                i.pointer.interact_pos(),
            )
        });
        if pressed {
            state.press = origin
                .filter(|o| !space && rect.contains(*o) && (response.hovered() || state.menu_open));
        }
        if released {
            if let (Some(o), Some(p)) = (state.press.take(), latest) {
                if o.distance(p) <= CYCLE_RADIUS {
                    if let Some(island) = state.choose(o, under(o), true) {
                        out.menu = Some((island, o));
                    }
                }
            }
        }
        out.hover = match pointer.filter(|_| !panning) {
            Some(p) => state.choose(p, under(p), false),
            None => None,
        };
        // 塗りと縁（状態か見え方が変わったときだけ作り直す）
        let key = Arc::as_ptr(data) as usize;
        let fresh = state.drawn.as_ref().is_some_and(|d| {
            d.data == key
                && d.priority == map.priority
                && d.rect == rect
                && d.zoom == state.zoom
                && d.center == state.center
        });
        if !fresh {
            let (under, over) = base_shapes(state, &ctx, data, &map.priority, &frame);
            state.drawn = Some(Drawn {
                data: key,
                priority: map.priority.clone(),
                rect,
                zoom: state.zoom,
                center: state.center,
                under,
                over,
            });
        }
        if let Some(d) = &state.drawn {
            painter.extend(d.under.iter().cloned());
        }
        // 重なったテクセル（キャンバスと同じテクスチャ）
        if let Some((texture, [x1, y1])) = map.overlap {
            let mut mesh = Mesh::with_texture(texture);
            let corners = [
                (Vec2::new(0.0, 0.0), pos2(0.0, 0.0)),
                (Vec2::new(x1, 0.0), pos2(1.0, 0.0)),
                (Vec2::new(x1, y1), pos2(1.0, 1.0)),
                (Vec2::new(0.0, y1), pos2(0.0, 1.0)),
            ];
            for (uv, tex) in corners {
                mesh.vertices.push(Vertex {
                    pos: frame.to_screen(uv),
                    uv: tex,
                    color: Color32::WHITE,
                });
            }
            mesh.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);
            painter.add(Shape::mesh(mesh));
        }
        if let Some(d) = &state.drawn {
            painter.extend(d.over.iter().cloned());
        }
        // 強調（メニューを開いているアイランド、無ければポインタか一覧の行のアイランド）
        if let Some(island) = map.menu.or(out.hover).or(list_hover) {
            if let Some(i) = data.islands.island(island) {
                let members: Vec<u32> = data
                    .islands
                    .members(i)
                    .iter()
                    .copied()
                    .filter(|&t| data.geometry.triangles()[t as usize].material == data.material)
                    .collect();
                painter.extend(triangles_mesh(data, &members, &frame, HOVER_FILL, None));
                let edges = data.edges_of(island);
                let shadow = Stroke::new(3.0, Color32::from_black_alpha(160));
                painter.extend(segments(edges, &frame, shadow));
                painter.extend(segments(edges, &frame, Stroke::new(1.6, t::TEXT)));
            }
        }
    }
    state.menu_open = map.menu.is_some();
    painter.rect_stroke(
        rect,
        0.0,
        Stroke::new(1.0, t::BORDER),
        egui::StrokeKind::Inside,
    );
    out
}
