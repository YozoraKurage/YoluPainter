//! マルチ 3D ビューの下書き（案 A・B・C とタブで隠した形）に共通の部品。
//!
//! 3D のモデルの所は、立方体の面と輪郭の線のダミー（本物の `OrbitCamera` で射影するので、視点ごとの見え方は本物と同じ向き）。
//! 軸の印は本物の `axis_gizmo::balls` の位置を使って描く。表示の種類のメニューの並びは、本物の `view3d::display::entries` と同じ。

use egui::{pos2, vec2, Align2, Color32, Id, Pos2, Rect, Sense, Shape, Stroke, Ui};
use yolu_app::ui::menu::{self, Entry as MenuEntry, PopupOutcome, PopupState};
use yolu_app::ui::theme as t;
use yolu_app::ui::widgets as w;
use yolu_app::view3d::axis_gizmo;
use yolu_app::view3d::shape_gizmo::{AXIS_X, AXIS_Y, AXIS_Z};
use yolu_core::geometry::{visible_height, AxisView, OrbitCamera, Projection};
use yolu_core::glam::Vec3;

use crate::page::SketchCtx;

// ───────── 表示の種類 ─────────

/// 3D ビューの表示の種類（マテリアル・中立・チャンネルだけ・メッシュマップだけ）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shade {
    Material,
    Neutral,
    /// 0 カラー・1 ラフネス・2 メタリック・3 ノーマル・4 エミッション・5 ハイト。
    Channel(usize),
    /// `MESH_MAPS` の番号。
    MeshMap(usize),
}

const CHANNELS: [(&str, &str); 6] = [
    ("カラー", "Color"),
    ("ラフネス", "Roughness"),
    ("メタリック", "Metallic"),
    ("ノーマル", "Normal"),
    ("エミッション", "Emission"),
    ("ハイト", "Height"),
];

const MESH_MAPS: [(&str, &str); 10] = [
    ("ワールド法線", "World Normal"),
    ("位置", "Position"),
    ("アンビエントオクルージョン", "Ambient Occlusion"),
    ("曲率", "Curvature"),
    ("厚み", "Thickness"),
    ("接空間法線", "Tangent Normal"),
    ("ハイト", "Height"),
    ("ID", "ID"),
    ("ベントノーマル", "Bent Normal"),
    ("不透明度", "Opacity"),
];

/// 今のテクスチャセットで焼いてある（選べる）メッシュマップの数（先頭から）。
const BAKED_MESH_MAPS: usize = 5;

impl Shade {
    /// メニュー・ドロップダウンの名前（本物の `shading_label` と同じ形）。
    pub fn name(self, ctx: &SketchCtx) -> String {
        match self {
            Shade::Material => ctx.pick("マテリアル (PBR)", "Material (PBR)").to_owned(),
            Shade::Neutral => ctx.pick("中立", "Neutral").to_owned(),
            Shade::Channel(c) => ctx.pick(CHANNELS[c].0, CHANNELS[c].1).to_owned(),
            Shade::MeshMap(k) => format!(
                "{}: {}",
                ctx.pick("メッシュマップ", "Mesh Map"),
                ctx.pick(MESH_MAPS[k].0, MESH_MAPS[k].1)
            ),
        }
    }

    /// `Values` に入れるための番号。
    pub fn code(self) -> usize {
        match self {
            Shade::Material => 0,
            Shade::Neutral => 1,
            Shade::Channel(c) => 2 + c,
            Shade::MeshMap(k) => 8 + k,
        }
    }

    pub fn from_code(code: usize) -> Shade {
        match code {
            0 => Shade::Material,
            1 => Shade::Neutral,
            2..=7 => Shade::Channel(code - 2),
            _ => Shade::MeshMap((code - 8).min(MESH_MAPS.len() - 1)),
        }
    }

    /// 狭い所（タブの見出し・ビューの隅の文字）の短い名前。
    pub fn short(self, ctx: &SketchCtx) -> String {
        match self {
            Shade::Material => ctx.pick("マテリアル", "Material").to_owned(),
            Shade::MeshMap(2) => "AO".to_owned(),
            Shade::MeshMap(k) => ctx.pick(MESH_MAPS[k].0, MESH_MAPS[k].1).to_owned(),
            other => other.name(ctx),
        }
    }

    /// ダミーの面の灰色（光なしの表示は一様、マテリアル・中立は光で濃淡を付ける。`k` は 0〜1 の明るさ）。
    fn tone(self, k: f32) -> Color32 {
        let g = match self {
            Shade::Material => 40.0 + k * 150.0,
            Shade::Neutral => 130.0 + k * 90.0,
            Shade::Channel(c) => [120.0, 214.0, 56.0, 150.0, 84.0, 100.0][c],
            Shade::MeshMap(m) => [
                128.0, 96.0, 186.0, 150.0, 110.0, 132.0, 90.0, 60.0, 144.0, 204.0,
            ][m],
        };
        Color32::from_gray(g as u8)
    }
}

/// 表示の種類のメニュー（本物の `view3d::display::entries` と同じ並び。選んでいる行に印）。
pub fn shading_entries(ctx: &SketchCtx, current: Shade) -> Vec<MenuEntry<Shade>> {
    let mut v = vec![
        MenuEntry::item(Shade::Material.name(ctx), Shade::Material)
            .radio(current == Shade::Material),
        MenuEntry::item(Shade::Neutral.name(ctx), Shade::Neutral).radio(current == Shade::Neutral),
        MenuEntry::Separator,
        MenuEntry::Heading(ctx.pick("チャンネルだけ", "Channel Only").to_owned()),
    ];
    for c in 0..CHANNELS.len() {
        let s = Shade::Channel(c);
        v.push(MenuEntry::item(s.name(ctx), s).radio(current == s));
    }
    v.push(MenuEntry::Separator);
    v.push(MenuEntry::Heading(
        ctx.pick("メッシュマップだけ", "Mesh Map Only").to_owned(),
    ));
    for (k, (ja, en)) in MESH_MAPS.iter().enumerate() {
        let s = Shade::MeshMap(k);
        let label = ctx.pick(ja, en);
        v.push(
            MenuEntry::item(label, s)
                .radio(current == s)
                .enabled(k < BAKED_MESH_MAPS),
        );
    }
    v
}

// ───────── ポップアップ ─────────

/// 開いているポップアップを 1 つ描き、選ばれた値を返す。`request` が Some のとき、その矩形の下に開く。
/// 撮る間は `still_anchor` の下に開いた形で出す（ウィンドウでは押して開く）。
pub fn popup<A: Clone>(
    ui: &Ui,
    ctx: &SketchCtx,
    id: Id,
    request: Option<Rect>,
    still_anchor: Option<Rect>,
    entries: &[MenuEntry<A>],
    keep: &[Rect],
) -> Option<A> {
    let mut state: Option<PopupState> = ui.data(|d| d.get_temp(id));
    if let Some(a) = request {
        state = Some(PopupState::new(ui.ctx(), a));
    } else if ctx.still && state.is_none() {
        state = still_anchor.map(|a| PopupState::new(ui.ctx(), a));
    }
    let mut chosen = None;
    if let Some(mut open) = state {
        let outcome = menu::show(ui.ctx(), id, &mut open, entries, keep);
        let keep_open = match outcome {
            PopupOutcome::Open => true,
            PopupOutcome::Chosen(a) => {
                chosen = Some(a);
                false
            }
            _ => false,
        };
        if keep_open || ctx.still {
            ui.data_mut(|d| d.insert_temp(id, open));
        } else {
            ui.data_mut(|d| d.remove::<PopupState>(id));
        }
    }
    chosen
}

/// アイコンの下に開くメニューの置き場所（本物の表示のメニューと同じ）。
pub fn below(icon: Rect) -> Rect {
    Rect::from_min_size(
        pos2(icon.left(), icon.bottom() + 2.0),
        vec2(icon.width(), 0.0),
    )
}

// ───────── 視点とダミーのモデル ─────────

/// ビューのカメラ（向きと投影）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cam {
    pub yaw: f32,
    pub pitch: f32,
    pub ortho: bool,
}

pub const PERSPECTIVE: Cam = Cam {
    yaw: 205.0,
    pitch: 18.0,
    ortho: false,
};
pub const PERSPECTIVE_B: Cam = Cam {
    yaw: 150.0,
    pitch: 24.0,
    ortho: false,
};
pub const BACK_ANGLE: Cam = Cam {
    yaw: 40.0,
    pitch: 14.0,
    ortho: false,
};

fn camera(cam: Cam, r: Rect) -> OrbitCamera {
    let aspect = (r.width() / r.height()).max(0.1);
    let distance = 4.8 * (1.0 / aspect).max(1.0);
    OrbitCamera {
        yaw: cam.yaw,
        pitch: cam.pitch,
        distance,
        model_radius: 0.9,
        projection: if cam.ortho {
            Projection::Orthographic {
                height: visible_height(distance),
            }
        } else {
            Projection::Perspective
        },
        ..OrbitCamera::default()
    }
}

/// ダミーのモデルの半分の大きさ（少し縦長の箱。視点ごとの見え方の違いが分かるように）。
const HALF: Vec3 = Vec3::new(0.45, 0.7, 0.45);

/// ビューの中身: 地面の格子と、灰色の立方体の面と輪郭の線。
fn model(painter: &egui::Painter, r: Rect, cam: Cam, shade: Shade) {
    w::fill(painter, r, t::CANVAS_BG);
    let painter = painter.with_clip_rect(r);
    let oc = camera(cam, r);
    let view = oc.view(r.width(), r.height());
    let project = |p: Vec3| -> Option<Pos2> {
        view.to_screen(p)
            .map(|s| pos2(r.left() + s.x, r.top() + s.y))
    };
    // 地面の格子
    let grid = Stroke::new(1.0, Color32::from_white_alpha(16));
    for i in -3..=3 {
        let o = i as f32 * 0.5;
        for (a, b) in [
            (Vec3::new(o, -HALF.y, -1.5), Vec3::new(o, -HALF.y, 1.5)),
            (Vec3::new(-1.5, -HALF.y, o), Vec3::new(1.5, -HALF.y, o)),
        ] {
            if let (Some(a), Some(b)) = (project(a), project(b)) {
                painter.line_segment([a, b], grid);
            }
        }
    }
    // 立方体（見える面だけ）
    let light = Vec3::new(0.4, 0.8, 0.5).normalize();
    let axes = [Vec3::X, Vec3::Y, Vec3::Z];
    for (i, normal_axis) in axes.iter().enumerate() {
        for sign in [-1.0f32, 1.0] {
            let n = *normal_axis * sign;
            let center = n * HALF[i];
            let to_eye = if cam.ortho {
                -view.forward
            } else {
                view.position - center
            };
            if n.dot(to_eye) <= 1e-4 {
                continue;
            }
            let (u, v) = (axes[(i + 1) % 3], axes[(i + 2) % 3]);
            let (hu, hv) = (HALF[(i + 1) % 3], HALF[(i + 2) % 3]);
            let corners: Vec<Pos2> = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
                .iter()
                .filter_map(|(a, b)| project(center + u * (*a * hu) + v * (*b * hv)))
                .collect();
            if corners.len() != 4 {
                continue;
            }
            let k = n.dot(light) * 0.5 + 0.5;
            painter.add(Shape::convex_polygon(
                corners,
                shade.tone(k),
                Stroke::new(1.2, Color32::from_white_alpha(95)),
            ));
        }
    }
}

/// 軸の印（本物の `axis_gizmo` と同じ並び・色。`scale` で小さくできる）。
fn gizmo(painter: &egui::Painter, cam: Cam, r: Rect, scale: f32) {
    let oc = camera(cam, r);
    let center = r.center();
    let color = |v: AxisView| match v {
        AxisView::Right | AxisView::Left => AXIS_X,
        AxisView::Top | AxisView::Bottom => AXIS_Y,
        AxisView::Front | AxisView::Back => AXIS_Z,
    };
    let letter = |v: AxisView| match v {
        AxisView::Right | AxisView::Left => "X",
        AxisView::Top | AxisView::Bottom => "Y",
        AxisView::Front | AxisView::Back => "Z",
    };
    for b in axis_gizmo::balls(&oc, center) {
        let c = color(b.view);
        let at = center + (b.at - center) * scale;
        if b.positive {
            painter.line_segment([center, at], Stroke::new(2.0 * scale.max(0.7), c));
            painter.circle_filled(at, 9.0 * scale, c);
            painter.text(
                at,
                Align2::CENTER_CENTER,
                letter(b.view),
                t::LABEL_BOLD.font(),
                Color32::from_rgb(16, 16, 18),
            );
        } else {
            painter.circle_filled(at, 6.5 * scale, c.gamma_multiply(0.4));
            painter.circle_stroke(at, 6.5 * scale, Stroke::new(1.2, c));
        }
    }
    painter.circle_filled(
        center,
        6.0 * scale,
        if cam.ortho { t::TEXT } else { t::PANEL_BG },
    );
    painter.circle_stroke(center, 6.0 * scale, Stroke::new(1.5, t::TEXT_DIM));
}

/// ビュー 1 つの中身（モデルと軸の印）。`gizmo_below` は軸の印を置く縦の位置（隅のアイコンの列の下の端。None なら印を出さない）。
pub fn draw_view(
    ui: &Ui,
    r: Rect,
    cam: Cam,
    shade: Shade,
    gizmo_below: Option<f32>,
    gizmo_scale: f32,
) {
    model(ui.painter(), r, cam, shade);
    if let Some(below) = gizmo_below {
        let g = axis_gizmo::rect(r, below);
        let side = axis_gizmo::SIZE * gizmo_scale;
        let g = Rect::from_min_size(
            pos2(r.right() - w::CORNER_MARGIN - side, g.top()),
            vec2(side, side),
        );
        if r.contains_rect(g) {
            gizmo(&ui.painter().with_clip_rect(r), cam, g, gizmo_scale);
        }
    }
}

/// フォーカスのあるビューの縁（細い青の線を内側に引く）。
pub fn focus_border(ui: &Ui, r: Rect) {
    w::outline(ui.painter(), r.shrink(0.75), t::ACCENT, 1.5, 0.0);
}

/// 画面全体を覆う（ドックの見出しと枠も隠す）面に、中身を描く。「Tab で UI を全部隠した」形や、ドックを自前で並べる案が使う。
pub fn cover(ui: &Ui, key: &'static str, content: impl FnOnce(&mut Ui, Rect)) {
    let screen = ui.ctx().content_rect();
    egui::Area::new(Id::new(("multiview.cover", key)))
        .order(egui::Order::Middle)
        .fixed_pos(screen.min)
        .constrain(false)
        .show(ui.ctx(), |ui| {
            ui.allocate_exact_size(screen.size(), Sense::hover());
            w::fill(ui.painter(), screen, t::WINDOW_BG);
            content(ui, screen);
        });
}

/// 隅のアイコンの列の id（ビューごとに別の名前が要る）。
pub const CORNER_IDS: [&str; 4] = ["mv0", "mv1", "mv2", "mv3"];

/// ビューどうしの隙間（境の線。背景の濃い色が見える）。
pub const GAP: f32 = 2.0;

/// `area` を、横に `cols` 列・縦に `rows` 行へ分ける（隙間は `GAP`）。左上から右へ、次の行へ。
pub fn grid(area: Rect, cols: usize, rows: usize) -> Vec<Rect> {
    let cw = (area.width() - GAP * (cols as f32 - 1.0)) / cols as f32;
    let ch = (area.height() - GAP * (rows as f32 - 1.0)) / rows as f32;
    let mut out = Vec::new();
    for row in 0..rows {
        for col in 0..cols {
            out.push(Rect::from_min_size(
                pos2(
                    area.left() + col as f32 * (cw + GAP),
                    area.top() + row as f32 * (ch + GAP),
                ),
                vec2(cw, ch),
            ));
        }
    }
    out
}

/// 本物の隅のアイコンの名前（ツールチップ）。
pub fn tip_shading(ctx: &SketchCtx) -> &'static str {
    ctx.pick("3D の表示", "3D shading")
}
pub fn tip_light(ctx: &SketchCtx) -> &'static str {
    ctx.pick(
        "光・環境・トーンマッピング",
        "Light, environment, tone mapping",
    )
}
pub fn tip_frame(ctx: &SketchCtx) -> &'static str {
    ctx.pick(
        "モデル全体が見える位置へ戻す",
        "Fit the whole model in view",
    )
}

/// 軸の印の大きさの比（小さいビューでは縮める）。
pub fn gizmo_scale(r: Rect) -> f32 {
    if r.height() >= 420.0 {
        1.0
    } else {
        0.75
    }
}

/// ビューの左上の、表示の種類のドロップダウン（押すと、その下に表示の種類のメニューが開く）。選ばれた値を返す。
/// `open_in_still` が真なら、撮る間はメニューを開いた形で出す。`key` はビューごとに別の名前（ポップアップの記憶に使う）。
pub fn shading_dropdown(
    ui: &mut Ui,
    ctx: &SketchCtx,
    view: Rect,
    key: (&'static str, usize),
    shade: Shade,
    open_in_still: bool,
) -> Option<Shade> {
    // 右上の隅のアイコンの列と軸の印にかからない幅
    let dd = Rect::from_min_size(
        pos2(view.left() + 8.0, view.top() + 8.0),
        vec2((view.width() - 16.0 - 130.0).clamp(110.0, 250.0), 24.0),
    );
    let (resp, boxr) = w::dropdown(
        ui,
        dd,
        ("mv.dd", key),
        None,
        &shade.name(ctx),
        Some(tip_shading(ctx)),
        true,
        0.0,
    );
    let request = resp.clicked().then(|| below(boxr));
    let anchor = open_in_still.then(|| below(boxr));
    let entries = shading_entries(ctx, shade);
    popup(
        ui,
        ctx,
        Id::new(("mv.shading-menu", key)),
        request,
        anchor,
        &entries,
        &[boxr],
    )
}
