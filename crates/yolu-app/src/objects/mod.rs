//! 編集のモードの選べる物（点の印で選ぶ）と、編集・ポーズのモードの G/R/S（`transform`）。
//!
//! - 選べる物: 投影の箱・デカール（塗りつぶしの投影が UV 以外）、グラデーションデカール（塗りつぶしのチャンネルの形のグラデーション）、
//!   フィルターの形（形のグラデーション・UV 以外の投影の画像の Generator。内容とマスクのスタック）、モデルの空間の点のグラデーションの点、
//!   3D パス（パス全体。今のモデルで描いたもの）、3D の定規（表示が入っているもの。表示の範囲によらない）。見えているレイヤーの物だけ。どのレイヤーの
//!   物も印を出し、押せばそのレイヤーを選ぶ。
//! - 印は物の中心（パスは線の真ん中、定規は 2 点の真ん中か中心）の小さな点（白・半径 6。乗せると大きく、選ぶと橙）。重なった印は、同じ所でもう一度押すと次の物。
//!   選んだ物は枠・線を橙で出し、形と定規はそのギズモの取っ手も出す（Q で隠す。G/R/S の途中は出さない。定規は移動の矢印と回す輪、端の点の四角で、
//!   大きさのつまみは無い）。
//! - 選びは 1 つ（複数を選んで一緒に動かすのは無い）。H で選んだ物の印を隠し、Alt+H で全部出す（画面だけ。文書は変えない）。
//! - 編集の Delete は選んだ物を消す（グラデーションデカール・フィルターの形・点・パス・定規。投影の置き場は消せない）。
//! - ポーズのモードの物はボーン（面を押して選ぶ。`view3d::gizmo`）。G/R/S は選んでいるボーンに当てる。

pub mod transform;

use egui::{pos2, Color32, Pos2, Rect, Shape as EguiShape, Stroke, Ui};
use yolu_core::fill_image::ProjectionMode;
use yolu_core::fill_points::PointSpace;
use yolu_core::glam::{Vec2, Vec3};
use yolu_core::paths::point_position;
use yolu_core::{Channel, FilterTarget, LayerId, LayerKind, LayerPath, RulerId, RulerSpace};

use crate::fillfx::gizmo::{self, Target};
use crate::lang::Lang;
use crate::mode::EditorMode;
use crate::notice::Source;
use crate::state::{Action, AppState};
use crate::view3d::shape_gizmo as sg;

pub use transform::Kind;

/// 印の半径・乗せたときの半径・掴める近さ（画面の点）。
pub const MARK_RADIUS: f32 = 6.0;
pub const MARK_HOVER_RADIUS: f32 = 8.0;
pub const GRAB_POINTS: f32 = 9.0;
/// 同じ所でもう一度押したとみなす近さ（画面の点）。
const SAME_PLACE: f32 = 4.0;
/// 選んだ物の色（形のギズモの枠と同じ橙）。
pub const SELECTED: Color32 = Color32::from_rgb(242, 148, 48);

/// 選べる物。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Object {
    /// 投影の箱・デカール・グラデーションデカール・フィルターの形（形のギズモの対象）。
    Shape(Target),
    /// モデルの空間の点のグラデーションの点。
    Point {
        layer: LayerId,
        channel: Channel,
        index: usize,
    },
    /// 3D パス（パス全体）。
    Path { layer: LayerId, id: u128 },
    /// 3D の定規（モデルの空間の定規。レイヤーが持つ文書の値）。
    Ruler { layer: LayerId, id: RulerId },
}

impl Object {
    pub fn layer(self) -> LayerId {
        match self {
            Object::Shape(t) => t.layer(),
            Object::Point { layer, .. }
            | Object::Path { layer, .. }
            | Object::Ruler { layer, .. } => layer,
        }
    }
}

/// 印 1 つ（物と、印を出す世界の点）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Marker {
    pub object: Object,
    pub world: Vec3,
}

/// G/R/S の刻み（Ctrl を押している間・Shift+Tab でスナップを常にしている間）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Steps {
    /// 移動（シーンの単位）。
    pub movement: f32,
    /// 回転（度）。
    pub rotation: f32,
    /// 拡縮（倍率）。
    pub scale: f32,
}

impl Default for Steps {
    fn default() -> Self {
        Steps {
            movement: 0.1,
            rotation: 5.0,
            scale: 0.1,
        }
    }
}

/// 編集・ポーズのモードの物の状態。アプリの状態で、.ylp には入れない。
#[derive(Default)]
pub struct ObjectsState {
    /// 選んでいる物（編集のモード）。
    pub selected: Option<Object>,
    /// H で隠した物（印と枠を出さない）。
    pub hidden: Vec<Object>,
    /// ポインタを乗せている印の物。
    pub hover: Option<Object>,
    /// 最後に印を押した所（同じ所でもう一度押すと、重なった次の物）。
    last_press: Option<Pos2>,
    /// G/R/S の途中（`PopupKind::Transform` のポップアップと一緒に持つ）。
    pub transform: Option<transform::Transform>,
    /// G/R/S を始める頼み（キーの処理がポインタの所から始める）。
    pub request: Option<Kind>,
    /// スナップを常にする（Shift+Tab。Ctrl を押している間は逆）。
    pub snap: bool,
    pub steps: Steps,
    /// ツールの帯の移動・回転・拡縮のドラッグを始める頼み（種類・押した所・入力。3D ビューの入力が同じフレームに始める）。
    pub drag_request: Option<(Kind, Pos2, gizmo::Source)>,
    /// ペンのドラッグで始めた G/R/S の、このフレームのペンの点と、離したか（3D ビューの入力が詰め、G/R/S が読む）。
    pub pen_at: Option<Pos2>,
    pub pen_lifted: bool,
}

/// 物の操作（`Action::Object`）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ObjectAction {
    /// G/R/S を始める。
    Transform(Kind),
    /// Alt+G/R/S: 位置・回転・大きさを既定へ（1 回の取り消し）。
    Reset(Kind),
    /// スナップを常にする・しない（Shift+Tab）。
    ToggleSnap,
    /// 選んだ物の印を隠す（H）。
    Hide,
    /// 隠した印を全部出す（Alt+H）。
    Reveal,
    /// 選んだ物を消す（Delete）。
    Delete,
    /// 物を選ぶ（None で外す）。
    Select(Option<Object>),
}

/// G/R/S の途中か。
pub fn transforming(app: &AppState) -> bool {
    app.objects.transform.is_some()
}

/// 物の今の形（形のギズモの対象。投影は UV のときは無い）。
pub fn shape_of(app: &AppState, t: Target) -> Option<sg::Shape> {
    if let Target::Projection(layer) = t {
        let l = app.doc.layer(layer)?;
        if l.kind() != LayerKind::Fill || l.projection().mode == ProjectionMode::Uv {
            return None;
        }
    }
    gizmo::shape(app, t)
}

/// 3D パス（今のモデルで描いた、見えているもの）と、その点の世界の位置。
pub fn path_points(app: &AppState, layer: LayerId, id: u128) -> Option<Vec<Vec3>> {
    path_points_with_normals(app, layer, id).map(|v| v.into_iter().map(|(at, _)| at).collect())
}

/// 3D パスの点の世界の位置と、その面の法線。
pub fn path_points_with_normals(
    app: &AppState,
    layer: LayerId,
    id: u128,
) -> Option<Vec<(Vec3, Vec3)>> {
    let model = app.view3d.full_model()?;
    let fingerprint = app.path_fingerprint(&model.geometry);
    let entry = app
        .doc
        .layer(layer)?
        .paths()
        .iter()
        .find(|e| e.path.id() == id)?;
    let LayerPath::Surface(p) = &entry.path else {
        return None;
    };
    if p.model_fingerprint != *fingerprint {
        return None;
    }
    p.points
        .iter()
        .map(|pt| point_position(&model.geometry, pt))
        .collect()
}

/// 折れ線の真ん中（長さの半分の所）。
pub fn polyline_middle(points: &[Vec3]) -> Option<Vec3> {
    let first = *points.first()?;
    let total: f32 = points.windows(2).map(|w| w[0].distance(w[1])).sum();
    if total <= 0.0 {
        return Some(first);
    }
    let mut left = total / 2.0;
    for w in points.windows(2) {
        let d = w[0].distance(w[1]);
        if d >= left && d > 0.0 {
            return Some(w[0].lerp(w[1], left / d));
        }
        left -= d;
    }
    points.last().copied()
}

/// 物の印の世界の点（物が無くなった・今は選べないなら None）。
pub fn marker_of(app: &AppState, object: Object) -> Option<Vec3> {
    match object {
        Object::Shape(t) => shape_of(app, t).map(|s| sg::world_center(&s, &gizmo::root())),
        Object::Point {
            layer,
            channel,
            index,
        } => {
            let g = app.doc.layer(layer)?.fill_points(channel)?;
            if g.space != PointSpace::Model {
                return None;
            }
            let p = g.points.get(index)?.position;
            Some(Vec3::new(p[0] as f32, p[1] as f32, p[2] as f32))
        }
        Object::Path { layer, id } => polyline_middle(&path_points(app, layer, id)?),
        Object::Ruler { layer, id } => {
            let r = ruler_of(app, layer, id)?;
            crate::rulers::edit3d::center(&r).map(|c| c.as_vec3())
        }
    }
}

/// 編集のモードで選べる 3D の定規（モデルの空間で、表示が入っているもの）。
pub fn ruler_of(app: &AppState, layer: LayerId, id: RulerId) -> Option<yolu_core::Ruler> {
    app.doc
        .layer(layer)?
        .rulers()
        .iter()
        .find(|r| r.id == id && r.space() == RulerSpace::Model && r.visible)
        .cloned()
}

/// レイヤーが見えているか（親のグループまで全部の目が開いている）。
pub fn shown(app: &AppState, layer: LayerId) -> bool {
    let mut at = Some(layer);
    let mut hops = 0;
    while let Some(id) = at {
        let Some(l) = app.doc.layer(id) else {
            return false;
        };
        if !l.visible() {
            return false;
        }
        at = l.parent();
        hops += 1;
        if hops > app.doc.layers().len() {
            return false;
        }
    }
    true
}

/// 物を今選べるか（印・選び・G/R/S・Alt+G/R/S・Delete が見る、ただ 1 つの決まり）: 物があり、そのレイヤーが親のグループまで見えていて、
/// H で隠していない。モデルが無ければ選べない。
pub fn selectable(app: &AppState, object: Object) -> bool {
    app.view3d.model.is_some()
        && !app.objects.hidden.contains(&object)
        && shown(app, object.layer())
        && marker_of(app, object).is_some()
}

/// 選んでいる物（今も選べるものだけ）。
pub fn selected(app: &AppState) -> Option<Object> {
    app.objects.selected.filter(|o| selectable(app, *o))
}

/// 選べる物の全部（見えているレイヤーの物。H で隠した物は除く）。モデルが無ければ空。
pub fn markers(app: &AppState) -> Vec<Marker> {
    if app.view3d.model.is_none() {
        return Vec::new();
    }
    let mut objects = Vec::new();
    for layer in app.doc.layers() {
        if !shown(app, layer.id()) {
            continue;
        }
        let id = layer.id();
        if layer.kind() == LayerKind::Fill {
            objects.push(Object::Shape(Target::Projection(id)));
            for (channel, _) in layer.fill_gradients() {
                objects.push(Object::Shape(Target::Gradient(id, channel)));
            }
            for (channel, g) in layer.fill_point_gradients() {
                if g.space == PointSpace::Model {
                    objects.extend((0..g.points.len()).map(|index| Object::Point {
                        layer: id,
                        channel,
                        index,
                    }));
                }
            }
        }
        for stack in [FilterTarget::Content, FilterTarget::Mask] {
            for f in app.doc.filters_of(id, stack).unwrap_or(&[]) {
                objects.push(Object::Shape(Target::Filter(id, f.id())));
            }
        }
        for entry in layer.paths() {
            if entry.visible && matches!(entry.path, LayerPath::Surface(_)) {
                objects.push(Object::Path {
                    layer: id,
                    id: entry.path.id(),
                });
            }
        }
        for r in layer.rulers() {
            if r.visible && r.space() == RulerSpace::Model {
                objects.push(Object::Ruler {
                    layer: id,
                    id: r.id,
                });
            }
        }
    }
    objects
        .into_iter()
        .filter(|o| !app.objects.hidden.contains(o))
        .filter_map(|object| marker_of(app, object).map(|world| Marker { object, world }))
        .collect()
}

/// 選んでいる物が今も選べるか（無くなった・隠したら外す）。
pub fn sync(app: &mut AppState) {
    if app.objects.selected.is_some() && selected(app).is_none() {
        app.objects.selected = None;
    }
}

/// 編集のモードで形のギズモ（取っ手）を出す対象: 選んでいる形（G/R/S の途中・Q で隠したときは無し）。
pub fn gizmo_target(app: &AppState) -> Option<Target> {
    if app.mode != EditorMode::Edit || transforming(app) || app.fillfx.handles_hidden {
        return None;
    }
    match selected(app)? {
        Object::Shape(t) => Some(t),
        Object::Ruler { layer, id } => Some(Target::Ruler(layer, id)),
        _ => None,
    }
}

fn screen(rect: Rect, v: Vec2) -> Pos2 {
    pos2(rect.left() + v.x, rect.top() + v.y)
}

/// 画面の点の近くの印（近い順。同じ近さなら並びの順）。
fn hits(app: &AppState, rect: Rect, at: Pos2) -> Vec<Object> {
    let view = app.view3d.camera.view(rect.width(), rect.height());
    let mut near: Vec<(f32, Object)> = markers(app)
        .into_iter()
        .filter_map(|m| {
            let s = screen(rect, view.to_screen(m.world)?);
            let d = s.distance(at);
            (d <= GRAB_POINTS).then_some((d, m.object))
        })
        .collect();
    near.sort_by(|a, b| a.0.total_cmp(&b.0));
    near.into_iter().map(|(_, o)| o).collect()
}

/// 物を選ぶ（そのレイヤーも選ぶ）。
pub fn select(app: &mut AppState, object: Option<Object>) {
    app.objects.selected = object;
    if let Some(o) = object {
        if app.selected_layer != Some(o.layer()) {
            app.select_single_layer(o.layer());
        }
        // 定規を選んだら、プロパティの「定規」の行も同じ定規を選ぶ
        if let Object::Ruler { layer, id } = o {
            app.rulers.selected = Some((layer, id));
        }
    }
}

/// 点のグラデーションの点を消した後: その点を選んでいたら外し（隣の点へ移らない）、後ろの点を選んでいた・隠していたら番号を詰める。
pub fn point_removed(app: &mut AppState, layer: LayerId, channel: Channel, index: usize) {
    let shift = |o: Object| -> Option<Object> {
        match o {
            Object::Point {
                layer: l,
                channel: c,
                index: i,
            } if l == layer && c == channel => match i.cmp(&index) {
                std::cmp::Ordering::Less => Some(o),
                std::cmp::Ordering::Equal => None,
                std::cmp::Ordering::Greater => Some(Object::Point {
                    layer,
                    channel,
                    index: i - 1,
                }),
            },
            _ => Some(o),
        }
    };
    app.objects.selected = app.objects.selected.and_then(shift);
    let hidden = std::mem::take(&mut app.objects.hidden);
    app.objects.hidden = hidden.into_iter().filter_map(shift).collect();
}

/// 編集のモードなら、その物を選ぶ（欄の「3D ビューで編集」・置いたデカールから。選べない物は選ばない）。
pub fn select_when_editing(app: &mut AppState, object: Object) {
    if app.mode == EditorMode::Edit && selectable(app, object) {
        select(app, Some(object));
    }
}

/// 編集のモードで左ボタン（かペンの接触）を押した: 印の上なら物を選び（重なっていれば、同じ所で押すたびに次の物）、選んだ形の取っ手の上なら
/// ドラッグを始め、何も無い所なら選びを外す。押しは受けたら true（描き始めない）。
pub fn press(app: &mut AppState, rect: Rect, at: Pos2, source: gizmo::Source) -> bool {
    if app.mode != EditorMode::Edit || app.is_stroking() {
        return false;
    }
    let hits = hits(app, rect, at);
    let same_place = app
        .objects
        .last_press
        .is_some_and(|p| p.distance(at) <= SAME_PLACE);
    app.objects.last_press = Some(at);
    let selected = selected(app);
    // ツールの帯の移動・回転・拡縮: 選んだ物の印（無ければ押した印の物を選んで）から、その種類の G/R/S のドラッグを始める
    if let Some(kind) = app.edit_tool.transform() {
        let o = selected
            .filter(|s| hits.contains(s))
            .or_else(|| hits.first().copied());
        if let Some(o) = o {
            select(app, Some(o));
            app.objects.drag_request = Some((kind, at, source));
            return true;
        }
    }
    let next = match selected.and_then(|s| hits.iter().position(|h| *h == s)) {
        // 選んでいる物の印の所: 重なった次の物（同じ所でもう一度押したとき。違う所なら取っ手を先に見る）
        Some(i) if hits.len() > 1 && same_place => Some(hits[(i + 1) % hits.len()]),
        Some(_) => None,
        None => hits.first().copied(),
    };
    if let Some(o) = next {
        select(app, Some(o));
        return true;
    }
    if gizmo::press(app, rect, at, source) {
        return true;
    }
    if hits.is_empty() {
        select(app, None);
    }
    true
}

/// 印と、選んだ物の枠・線を 3D ビューへ重ねる（編集のモード）。`pointer` はポインタ（乗せた印を大きくする）。
pub fn draw(ui: &Ui, app: &mut AppState, rect: Rect, pointer: Option<Pos2>) {
    sync(app);
    let view = app.view3d.camera.view(rect.width(), rect.height());
    let painter = ui.painter_at(rect);
    let line = |points: Vec<Pos2>, width: f32| {
        painter.add(EguiShape::line(
            points.clone(),
            Stroke::new(width + 2.0, Color32::from_black_alpha(115)),
        ));
        painter.add(EguiShape::line(points, Stroke::new(width, SELECTED)));
    };
    // 選んだ物の枠・線（取っ手を出している形は、ギズモが枠も描く）
    if let Some(o) = selected(app) {
        {
            match o {
                Object::Shape(t) if gizmo::target(app) != Some(t) => {
                    if let Some(s) = shape_of(app, t) {
                        for l in sg::outline(&s, &gizmo::root(), &view) {
                            if !l.filled {
                                line(l.points.iter().map(|p| screen(rect, *p)).collect(), l.width);
                            }
                        }
                    }
                }
                Object::Path { layer, id } => {
                    let world = match &app.objects.transform {
                        Some(t) => t.path_preview().unwrap_or_default(),
                        None => path_points(app, layer, id).unwrap_or_default(),
                    };
                    let pts: Vec<Pos2> = world
                        .iter()
                        .filter_map(|w| view.to_screen(*w).map(|s| screen(rect, s)))
                        .collect();
                    if pts.len() > 1 {
                        line(pts, 2.0);
                    }
                }
                Object::Ruler { layer, id } => {
                    if let Some(r) = ruler_of(app, layer, id) {
                        crate::rulers::draw3d::paint_selected(&painter, app, rect, &r);
                        if gizmo::target(app).is_some() {
                            let hover = match app.fillfx.hover {
                                sg::Handle::EndA => Some(crate::rulers::edit3d::End::A),
                                sg::Handle::EndB => Some(crate::rulers::edit3d::End::B),
                                _ => None,
                            };
                            crate::rulers::draw3d::paint_squares(&painter, app, rect, &r, hover);
                        }
                    }
                }
                _ => {}
            }
        }
    }
    let busy = transforming(app) || app.fillfx.drag.is_some() || app.view3d.input.nav.is_some();
    let markers = markers(app);
    let hover = pointer.filter(|_| !busy).and_then(|p| {
        markers
            .iter()
            .filter_map(|m| Some((screen(rect, view.to_screen(m.world)?).distance(p), m.object)))
            .filter(|(d, _)| *d <= GRAB_POINTS)
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, o)| o)
    });
    app.objects.hover = hover;
    for m in &markers {
        // G/R/S の途中のパスは、動かした線の真ん中に印を出す
        let world = match (&app.objects.transform, m.object) {
            (Some(t), Object::Path { .. }) if Some(m.object) == app.objects.selected => t
                .path_preview()
                .and_then(|w| polyline_middle(&w))
                .unwrap_or(m.world),
            _ => m.world,
        };
        let Some(s) = view.to_screen(world) else {
            continue;
        };
        let at = screen(rect, s);
        let selected = Some(m.object) == app.objects.selected;
        let radius = if hover == Some(m.object) {
            MARK_HOVER_RADIUS
        } else {
            MARK_RADIUS
        };
        painter.circle_filled(at, radius + 1.5, Color32::from_black_alpha(150));
        painter.circle_filled(at, radius, if selected { SELECTED } else { Color32::WHITE });
    }
}

/// 物の名前（知らせの文に使う）。
fn object_name(lang: Lang, o: Object) -> &'static str {
    match o {
        Object::Shape(Target::Projection(_)) => lang.pick("投影の置き場", "Projection placement"),
        Object::Shape(Target::Gradient(..)) => {
            lang.pick("グラデーションデカール", "Gradient decal")
        }
        Object::Shape(Target::Filter(..)) => lang.pick("フィルターの形", "Filter shape"),
        Object::Point { .. } => lang.pick("点", "Point"),
        Object::Path { .. } => lang.pick("パス", "Path"),
        Object::Ruler { .. } | Object::Shape(Target::Ruler(..)) => lang.pick("定規", "Ruler"),
    }
}

/// 選んでいる物が無いときの断り。
fn nothing_selected(lang: Lang) -> &'static str {
    lang.pick("選んだ物がありません", "Nothing is selected")
}

impl AppState {
    /// 物の操作を当てる（`Action::Object`）。
    pub fn objects_apply(&mut self, action: ObjectAction) {
        let lang = self.lang;
        match action {
            ObjectAction::Transform(kind) => transform::request(self, kind),
            ObjectAction::Reset(kind) => transform::reset(self, kind),
            ObjectAction::ToggleSnap => self.objects.snap = !self.objects.snap,
            ObjectAction::Select(o) => select(self, o),
            ObjectAction::Hide => {
                if let Some(o) = selected(self) {
                    self.objects.hidden.push(o);
                }
                self.objects.selected = None;
            }
            ObjectAction::Reveal => self.objects.hidden.clear(),
            ObjectAction::Delete => {
                if self.is_stroking() || transforming(self) {
                    self.refuse(Source::Edit, crate::lang::refusals::during_stroke(lang));
                    return;
                }
                let Some(o) = selected(self) else {
                    let text = lang.with_reason(
                        lang.pick("削除できません", "Cannot delete"),
                        nothing_selected(lang),
                    );
                    self.refuse(Source::Edit, text);
                    return;
                };
                delete(self, o);
            }
        }
    }
}

/// 選んだ物を消す（今の消す口を使う。消せない物は理由を知らせる）。
fn delete(app: &mut AppState, o: Object) {
    let lang = app.lang;
    let what = lang.pick(
        format!("{}を削除できません", object_name(lang, o)),
        format!("Cannot delete the {}", object_name(lang, o).to_lowercase()),
    );
    match o {
        Object::Shape(Target::Projection(_)) => {
            let text = lang.with_reason(
                what,
                lang.pick(
                    "塗りつぶしレイヤーの投影です",
                    "it is the fill layer's projection",
                ),
            );
            app.refuse(Source::FillLayer, text);
        }
        Object::Shape(Target::Gradient(layer, channel)) => {
            app.apply(Action::Fill(crate::fillfx::FillOp::Gradient {
                layer,
                channel,
                gradient: None,
                coalesce: false,
            }));
        }
        Object::Shape(Target::Filter(layer, id)) => {
            app.apply(Action::Fx(crate::fx::FxOp::Remove { layer, id }));
        }
        Object::Point {
            layer,
            channel,
            index,
        } => {
            if let Some(reason) = app.read_only_reason() {
                let text = crate::lang::refusals::read_only_set(lang, reason);
                app.refuse(Source::FillLayer, text);
                return;
            }
            // 今の点を消す口（最後の点は断る。消したら `point_removed` が選びを外し、番号を詰める）
            crate::fillfx::points::delete_point(app, layer, channel, index);
        }
        // 定規の物は `Object::Ruler`（形のギズモの対象 `Target::Ruler` を `Object::Shape` に包むことは無いが、同じ消し方にしておく）
        Object::Ruler { layer, id } | Object::Shape(Target::Ruler(layer, id)) => {
            app.apply(Action::Ruler(crate::rulers::RulerAction::Delete {
                owner: layer,
                ids: vec![id],
            }));
        }
        Object::Path { layer, id } => {
            if let Some(reason) = app.read_only_reason() {
                let text = crate::lang::refusals::read_only_set(lang, reason);
                app.refuse(Source::Path, text);
                return;
            }
            let Some(entries) = app.doc.layer(layer).map(|l| l.paths().to_vec()) else {
                return;
            };
            let kept: Vec<_> = entries.into_iter().filter(|e| e.path.id() != id).collect();
            app.doc.end_coalescing();
            // パスの一覧の口（編集中のパス・選んだ点を外し、面から外れた標本を知らせる）
            if app.path_commit_list(layer, kept, None) {
                app.objects.selected = None;
            }
        }
    }
    sync(app);
}

#[cfg(test)]
mod tests;
