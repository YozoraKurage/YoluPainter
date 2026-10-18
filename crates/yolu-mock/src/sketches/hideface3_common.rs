//! 面を隠す（5 回目）の下書きが共通で使う部品: ハイドのモードの画面（左上のモードのドロップダウン・左のツールの帯・3D ビュー）、3D ビューの中の
//! ボーンの丸、3D ビューの右から出る「ハイド」の引き出し、左下のシェルフ（ハイドとポーズのプリセットを選ぶ）、ポーズのモードの回転のギズモ。
//!
//! モードのドロップダウンの「ハイド」の行と、ハイドのモードの左のツールの帯は、本物にまだ無いので下書きの中で描く（見た目は本物の
//! `mode::dropdown`・`w::tool_button` に合わせる）。ボーンの丸は本物の編集のモードの物の印（`objects::draw`）と同じ見た目で、
//! 回転のギズモの色と大きさは本物の `view3d::gizmo` の定数を使う。

use egui::{pos2, vec2, Color32, Id, Pos2, Rect, Sense, Shape, Stroke, Ui, UiBuilder};
use yolu_app::mode::{self, EditorMode};
use yolu_app::objects::SELECTED;
use yolu_app::panels::properties::percent_row;
use yolu_app::ui::menu::{self, Entry as MenuEntry, PopupOutcome, PopupState};
use yolu_app::ui::scroll::Scroll;
use yolu_app::ui::theme as t;
use yolu_app::ui::widgets::{self as w, Align, CornerIcon, Rows};
use yolu_app::view3d::gizmo::{AXIS_COLORS, RING_RADIUS, VIEW_RING_RADIUS};

use super::hideface_common::{self as common, Figure, Look};
use crate::page::SketchCtx;

/// モード（ハイドのモードとポーズのモードで、ボーンの丸を出す）。
#[derive(Clone, Copy, PartialEq)]
pub enum Mode {
    Hide,
    Pose,
}

/// シェルフの見せ方（撮る間の形）。
#[derive(Clone, Copy, PartialEq)]
pub enum ShelfLook {
    /// たたんだ形（目と ▽ だけ）。
    Closed,
    /// 開いた形。
    Open,
    /// 開いて、ハイドのプリセットの一覧を開いた形。
    OpenWithList,
}

const ROW: f32 = t::ROW_HEIGHT;
const ICON_W: f32 = 22.0;

// ───────── 3D ビュー 1 つの状態 ─────────

/// 3D ビュー 1 つの見せ方。
pub struct Pane {
    /// ビューごとに別の名前（隅のアイコン・シェルフ・引き出しの記憶に使う）。
    pub key: &'static str,
    pub mode: Mode,
    /// 隠している UV アイランド（このビューに隠すを当てているとき、描かない）。
    pub hidden: Vec<usize>,
    /// ボーンで隠した項目（Hand.L）があるか。
    pub bone_hidden: bool,
    /// 選んでいる UV アイランドと、ポインターが乗っている UV アイランド（ハイドのモードだけ）。
    pub selected_faces: Vec<usize>,
    pub pointed: Option<usize>,
    /// 選んでいるボーン（`BONES` の番号）。
    pub selected_bones: Vec<usize>,
    /// このビューに隠すを当てているか（シェルフの目の切り替えの初めの値）。
    pub applied: bool,
    pub shelf: ShelfLook,
    pub hide_preset: usize,
    pub pose_preset: usize,
    /// 引き出しを開いた形で始めるか（ハイドのモードだけ）。
    pub drawer_open: bool,
    /// ズーム（1 が全体が収まる大きさ）。
    pub zoom: f32,
    /// 右上の列に「3D の表示」を出すか（マルチ 3D ビューでは左上のドロップダウンへ移っている）。
    pub shading_icon: bool,
}

pub struct Opts {
    pub pane: Pane,
    /// モードのドロップダウンを開いた形で出す。
    pub mode_menu: bool,
}

/// 画面全体（オプションバー・左のツールの帯・3D ビュー 1 つ）。
pub fn draw(ui: &mut Ui, ctx: &mut SketchCtx, opts: Opts) {
    let area = ui.max_rect();
    ui.advance_cursor_after_rect(area);
    let view = chrome(
        ui,
        ctx,
        area,
        opts.pane.mode,
        opts.mode_menu,
        !opts.pane.selected_faces.is_empty(),
    );
    pane(ui, ctx, view, &opts.pane);
}

/// 上のオプションバー（左端に今のモード）と左のツールの帯を描き、残りの 3D ビューの矩形を返す。
pub fn chrome(
    ui: &mut Ui,
    ctx: &mut SketchCtx,
    area: Rect,
    mode: Mode,
    mode_menu: bool,
    can_hide: bool,
) -> Rect {
    let p = ui.painter().clone();
    w::fill(&p, area, t::WINDOW_BG);
    let bar = Rect::from_min_size(area.min, vec2(area.width(), t::OPTIONS_BAR_HEIGHT));
    w::fill(&p, bar, t::PANEL_BG);
    w::hline(&p, bar.left(), bar.right(), bar.bottom() - 1.0, t::BORDER);
    let below = Rect::from_min_max(pos2(area.left(), bar.bottom()), area.max);
    let strip = Rect::from_min_size(below.min, vec2(t::TOOL_STRIP_WIDTH, below.height()));
    w::fill(&p, strip, t::PANEL_BG);
    match mode {
        Mode::Hide => {
            mode_dropdown(ui, ctx, bar, mode_menu);
            hide_strip(ui, ctx, strip, can_hide);
        }
        Mode::Pose => {
            // ポーズのモードは本物のドロップダウンと帯
            ctx.app.mode = EditorMode::Pose;
            let _ = mode::dropdown(ui, &mut ctx.app, bar, bar.left() + 8.0);
            mode::edit_strip(ui, &mut ctx.app, strip, strip.bottom());
        }
    }
    Rect::from_min_max(pos2(strip.right(), below.top()), area.max)
}

// ───────── ボーン ─────────

/// 箱の中を通るボーンの連なり（名前と、箱の並びの座標での関節の位置。親は 1 つ前）。
const BONES: [(&str, f32, f32); 7] = [
    ("Hips", -0.42, 0.45),
    ("Spine", -0.30, 0.41),
    ("Chest", -0.17, 0.36),
    ("Shoulder.L", -0.02, 0.33),
    ("UpperArm.L", 0.08, 0.38),
    ("LowerArm.L", 0.12, 0.26),
    ("Hand.L", 0.13, 0.14),
];

/// 丸の世界での半径（箱の並びの幅を 1.5 m とした、1.2 cm）。画面ではズームについて変わる。
const BONE_WORLD_RADIUS: f32 = 0.008;
const BONE_MIN: f32 = 3.0;
const BONE_MAX: f32 = 9.0;

fn bone_screen(figure: &Figure, i: usize) -> Pos2 {
    figure.screen_point(pos2(BONES[i].1, BONES[i].2))
}

/// 丸の画面での半径（3〜9 点。乗せたときは + 2）。
fn bone_radius(figure: &Figure, hover: bool) -> f32 {
    let r = (BONE_WORLD_RADIUS * figure.scale()).clamp(BONE_MIN, BONE_MAX);
    if hover {
        r + 2.0
    } else {
        r
    }
}

/// ボーンの丸: 塗りの無い半透明の輪（白 60%・外側の黒い縁 35%。選んだ物は橙 85%、乗せたときはどちらも少し濃く）。半径はズームについて変わる。
fn draw_bones(p: &egui::Painter, figure: &Figure, selected: &[usize], hover: Option<usize>) {
    for i in 0..BONES.len() {
        let at = bone_screen(figure, i);
        let hot = hover == Some(i);
        let radius = bone_radius(figure, hot);
        let color = if selected.contains(&i) {
            SELECTED.gamma_multiply(if hot { 1.0 } else { 0.85 })
        } else {
            Color32::WHITE.gamma_multiply(if hot { 0.85 } else { 0.6 })
        };
        // 輪の線の太さは 2 点。黒い縁は輪の外側に 1.5 点
        p.circle_stroke(
            at,
            radius + 1.75,
            Stroke::new(1.5, Color32::from_black_alpha(if hot { 120 } else { 90 })),
        );
        p.circle_stroke(at, radius, Stroke::new(2.0, color));
    }
}

/// 回転のギズモの輪 1 つ（楕円。後ろ半分を薄く）。`tilt` は傾き（ラジアン）。
fn ring(p: &egui::Painter, centre: Pos2, rx: f32, ry: f32, tilt: f32, color: Color32) {
    let at = |a: f32| {
        let (x, y) = (rx * a.cos(), ry * a.sin());
        centre
            + vec2(
                x * tilt.cos() - y * tilt.sin(),
                x * tilt.sin() + y * tilt.cos(),
            )
    };
    let steps = 64;
    for front in [false, true] {
        let c = if front {
            color
        } else {
            color.gamma_multiply(0.3)
        };
        let mut run: Vec<Pos2> = Vec::new();
        for k in 0..=steps {
            let a = std::f32::consts::TAU * k as f32 / steps as f32;
            if (a.sin() >= 0.0) == front {
                run.push(at(a));
            } else {
                if run.len() > 1 {
                    p.add(Shape::line(std::mem::take(&mut run), Stroke::new(2.0, c)));
                }
                run.clear();
            }
        }
        if run.len() > 1 {
            p.add(Shape::line(run, Stroke::new(2.0, c)));
        }
    }
}

/// 選んだボーンの回転のギズモ（X・Y・Z の輪 半径 64、画面の輪 半径 80。本物の `view3d::gizmo::draw` と同じ色・半径）と、親と子へ引く線。
fn draw_rotation_gizmo(p: &egui::Painter, figure: &Figure, bone: usize) {
    let centre = bone_screen(figure, bone);
    let line = Stroke::new(1.5, Color32::from_white_alpha(170));
    let mut others = vec![];
    if bone > 0 {
        others.push(bone - 1);
    }
    if bone + 1 < BONES.len() {
        others.push(bone + 1);
    }
    for o in others {
        let e = bone_screen(figure, o);
        p.line_segment([centre, e], line);
        p.circle_filled(e, 2.5, Color32::from_white_alpha(170));
    }
    // 輪（X・Y・Z・画面）。向きの違う楕円で立体に見せる
    ring(
        p,
        centre,
        RING_RADIUS * 0.34,
        RING_RADIUS,
        0.0,
        AXIS_COLORS[0],
    );
    ring(
        p,
        centre,
        RING_RADIUS,
        RING_RADIUS * 0.34,
        0.0,
        AXIS_COLORS[1],
    );
    ring(
        p,
        centre,
        RING_RADIUS,
        RING_RADIUS * 0.62,
        0.7,
        AXIS_COLORS[2],
    );
    ring(
        p,
        centre,
        VIEW_RING_RADIUS,
        VIEW_RING_RADIUS,
        0.0,
        AXIS_COLORS[3],
    );
    p.circle_filled(centre, 3.5, Color32::from_rgb(250, 210, 70));
}

// ───────── モードのドロップダウン ─────────

/// ドロップダウンの幅（本物の `mode::DROPDOWN_WIDTH`）。
const DROPDOWN_WIDTH: f32 = 132.0;

/// 本物の `mode::dropdown` と同じ見た目で、今のモード「ハイド」を出す。`open` なら、ペイント・編集・ポーズ・ハイドの 4 行のメニューを開く。
fn mode_dropdown(ui: &mut Ui, ctx: &SketchCtx, bar: Rect, open: bool) {
    let r = Rect::from_min_size(
        pos2(bar.left() + 8.0, bar.top() + 5.0),
        vec2(DROPDOWN_WIDTH, bar.height() - 10.0),
    );
    let p = ui.painter().clone();
    let fill = if open { t::ACCENT_DIM } else { t::PANEL_HEADER };
    w::rounded(&p, r, fill, 4.0);
    let color = if open { Color32::WHITE } else { t::TEXT };
    w::icon(
        &p,
        Rect::from_min_size(pos2(r.left() + 6.0, r.top()), vec2(20.0, r.height())),
        "visibility_off",
        color,
        18.0,
    );
    w::text(
        &p,
        Rect::from_min_max(
            pos2(r.left() + 32.0, r.top()),
            pos2(r.right() - 22.0, r.bottom()),
        ),
        ctx.pick("ハイド", "Hide"),
        t::LABEL.with_color(color),
        Align::Left,
    );
    w::icon(
        &p,
        Rect::from_min_size(pos2(r.right() - 22.0, r.top()), vec2(18.0, r.height())),
        "expand_more",
        if open { color } else { t::TEXT_DIM },
        16.0,
    );
    if open {
        let entries: Vec<MenuEntry<()>> = vec![
            MenuEntry::item(ctx.pick("ペイント", "Paint"), ()).icon("paint_brush", false),
            MenuEntry::item(ctx.pick("編集", "Edit"), ()).icon("arrow_move", false),
            MenuEntry::item(ctx.pick("ポーズ", "Pose"), ()).icon("accessibility", false),
            MenuEntry::item(ctx.pick("ハイド", "Hide"), ()).icon("visibility_off", true),
        ];
        let anchor = Rect::from_min_size(pos2(r.left(), r.bottom() + 2.0), vec2(r.width(), 0.0));
        still_menu(
            ui,
            ctx,
            Id::new("hideface5.mode-menu"),
            anchor,
            &entries,
            r.width(),
        );
    }
}

/// 開いたままのメニュー（撮る間の絵。ウィンドウでは出さない）。
fn still_menu<A: Clone>(
    ui: &Ui,
    ctx: &SketchCtx,
    id: Id,
    anchor: Rect,
    entries: &[MenuEntry<A>],
    min_width: f32,
) -> Option<A> {
    let mut state: Option<PopupState> = ui.data(|d| d.get_temp(id));
    if state.is_none() && ctx.still {
        state = Some(PopupState::new(ui.ctx(), anchor).with_min_width(min_width));
    }
    let mut open = state?;
    let outcome = menu::show(ui.ctx(), id, &mut open, entries, &[]);
    let mut chosen = None;
    match outcome {
        PopupOutcome::Chosen(a) => {
            chosen = Some(a);
            ui.data_mut(|d| d.remove::<PopupState>(id));
        }
        PopupOutcome::Open => {
            ui.data_mut(|d| d.insert_temp(id, open));
        }
        _ => {
            ui.data_mut(|d| d.remove::<PopupState>(id));
        }
    }
    chosen
}

// ───────── ハイドのモードの左のツールの帯 ─────────

/// 左のツールの帯: UV アイランドを選ぶ・四角で選ぶ（選ぶツール 2 つ）、すべて選択・選択を解除、隠す・すべて表示（名前とキーはツールチップ）。
/// 帯のアイコンは今ある物だけ（`tools/id-select`・`tools/select-rectangle`・`select_all`・`deselect`・`visibility_off`・`visibility`）。
fn hide_strip(ui: &mut Ui, ctx: &SketchCtx, strip: Rect, can_hide: bool) {
    let p = ui.painter().clone();
    let mut y = strip.top() + 6.0;
    let button = |y: f32| {
        Rect::from_min_size(
            pos2(strip.left() + 5.0, y),
            vec2(strip.width() - 10.0, 32.0),
        )
    };
    let _ = w::tool_button(
        ui,
        button(y),
        "id-select",
        ctx.pick("UV アイランドを選ぶ", "Select UV Island"),
        true,
    );
    y += 34.0;
    let _ = w::tool_button(
        ui,
        button(y),
        "select-rectangle",
        ctx.pick("四角で選ぶ", "Rectangle Select"),
        false,
    );
    y += 38.0;
    w::hline(
        &p,
        strip.left() + 8.0,
        strip.right() - 8.0,
        y - 3.0,
        t::SEPARATOR,
    );
    let selecting = can_hide;
    let actions: [(&str, &str, &str, bool); 4] = [
        (
            "select_all",
            "すべて選択（Ctrl+A）",
            "Select All (Ctrl+A)",
            true,
        ),
        (
            "deselect",
            "選択を解除（Ctrl+D）",
            "Deselect (Ctrl+D)",
            selecting,
        ),
        (
            "visibility_off",
            "選んだ面を隠す（H）",
            "Hide Selected Faces (H)",
            selecting,
        ),
        (
            "visibility",
            "すべて表示（Alt+H）",
            "Show All (Alt+H)",
            true,
        ),
    ];
    for (k, (icon, ja, en, enabled)) in actions.into_iter().enumerate() {
        if k == 2 {
            y += 4.0;
            w::hline(
                &p,
                strip.left() + 8.0,
                strip.right() - 8.0,
                y - 3.0,
                t::SEPARATOR,
            );
        }
        let _ = w::icon_button(
            ui,
            button(y),
            ("hideface5.strip", k),
            icon,
            ctx.pick(ja, en),
            false,
            enabled,
            20.0,
        );
        y += 34.0;
    }
}

// ───────── 3D ビュー ─────────

/// 隠している物の一覧（ボーンで隠した物と、UV アイランドで隠した物）。
struct Entry {
    icon: &'static str,
    name: String,
    status: String,
}

fn entries(ctx: &SketchCtx, p: &Pane) -> Vec<Entry> {
    let children = ctx.pick("・子", " +children");
    let mut list = Vec::new();
    if p.bone_hidden {
        list.push(Entry {
            icon: "accessibility",
            name: "Hand.L".to_owned(),
            status: format!("50%{children}"),
        });
    }
    let mut islands = p.hidden.clone();
    islands.sort_unstable();
    for island in islands {
        list.push(Entry {
            icon: "uv_wireframe",
            name: format!("{} {}", ctx.pick("UV アイランド", "UV Island"), island + 1),
            status: String::new(),
        });
    }
    list
}

/// 3D ビュー 1 つ（モデル・ボーンの丸・右上のアイコン・左下のシェルフ・ハイドのモードの右の引き出し）。
pub fn pane(ui: &mut Ui, ctx: &mut SketchCtx, view: Rect, pn: &Pane) {
    let p = ui.painter_at(view);
    w::fill(&p, view, t::CANVAS_BG);
    let figure = Figure::fit_zoom(view, pn.zoom);
    let hide_mode = pn.mode == Mode::Hide;
    let hidden: Vec<usize> = if pn.applied {
        pn.hidden.clone()
    } else {
        vec![]
    };
    let is_hidden = |i: usize| hidden.contains(&i);
    let look = |i: usize| {
        if !hide_mode {
            Look::Normal
        } else if pn.selected_faces.contains(&i) {
            Look::Hover
        } else if pn.pointed == Some(i) {
            Look::Pointed
        } else {
            Look::Normal
        }
    };
    common::draw_figure(&p, &figure, &is_hidden, &look, hide_mode && pn.applied);

    // ボーンの丸（メッシュの上に重ねる）。ポーズのモードは、選んだボーンの回転のギズモも重ねる
    draw_bones(&p, &figure, &pn.selected_bones, None);
    if !hide_mode {
        if let Some(bone) = pn.selected_bones.first() {
            draw_rotation_gizmo(&p, &figure, *bone);
        }
    }
    if hide_mode {
        common::draw_mode_frame(&p, view);
        if let Some(island) = pn.pointed {
            common::draw_pointer(&p, figure.center_of(island) + vec2(10.0, 40.0));
        }
    }

    // 右上の表示のアイコンの並び（今あるアイコンだけ）
    let lang = ctx.lang;
    let mut items = Vec::new();
    if pn.shading_icon {
        items.push(CornerIcon::new(
            "shading",
            "texture",
            lang.pick("3D の表示", "3D Shading"),
        ));
    }
    items.push(CornerIcon::new(
        "settings",
        "light_mode",
        lang.pick("光と環境", "Light and Environment"),
    ));
    items.push(CornerIcon::new(
        "frame",
        "target",
        lang.pick("全体を表示", "Frame Model"),
    ));
    let _ = w::corner_icons(ui, pn.key, view, &items);
    let drawer_top = w::corner_bottom(view, items.len()) + 4.0;

    // 左下のシェルフ（ペイント・ハイド・ポーズのモードで出す。編集のモードでは出さない）
    shelf(
        ui,
        ctx,
        &Shelf {
            key: pn.key,
            anchor: pos2(view.left() + 10.0, view.bottom() - 10.0),
            look: pn.shelf,
            applied: pn.applied,
            hide: pn.hide_preset,
            pose: pn.pose_preset,
        },
    );

    if hide_mode {
        drawer(ui, ctx, view, drawer_top, pn);
    }
}

// ───────── 3D ビューの右から出る「ハイド」の引き出し ─────────

const DRAWER_WIDTH: f32 = 280.0;
const HANDLE_WIDTH: f32 = 14.0;
const HANDLE_HEIGHT: f32 = 48.0;

/// つまみの上の「‹」「›」（本物のアイコンに左向きの矢印が無いので、線で描く）。
fn chevron(p: &egui::Painter, r: Rect, left: bool, color: Color32) {
    let c = r.center();
    let d = if left { -1.0 } else { 1.0 };
    let pts = vec![
        c + vec2(-2.5 * d, -5.0),
        c + vec2(2.5 * d, 0.0),
        c + vec2(-2.5 * d, 5.0),
    ];
    p.add(Shape::line(pts, Stroke::new(1.5, color)));
}

/// 引き出し: 閉じた形はビューの右端の縦の真ん中につまみだけ（「‹」）、開いた形は幅 280 の欄が右端から出て、つまみは欄の左の縁に付いたまま（「›」）。
/// 欄の上端は右上のアイコンの列の下、下端はビューの下から少し空け、あふれたらスクロール。
fn drawer(ui: &mut Ui, ctx: &mut SketchCtx, view: Rect, top: f32, pn: &Pane) {
    let open_key = format!("hideface5.drawer.open.{}", pn.key);
    let open = ctx.values.bool(&open_key, pn.drawer_open);
    let bottom = view.bottom() - 12.0;
    let panel = Rect::from_min_max(
        pos2(view.right() - DRAWER_WIDTH, top),
        pos2(view.right(), bottom),
    );
    let cy = view
        .center()
        .y
        .clamp(top + HANDLE_HEIGHT / 2.0, bottom - HANDLE_HEIGHT / 2.0);
    let right_edge = if open { panel.left() } else { view.right() };
    let handle = Rect::from_min_size(
        pos2(right_edge - HANDLE_WIDTH, cy - HANDLE_HEIGHT / 2.0),
        vec2(HANDLE_WIDTH, HANDLE_HEIGHT),
    );
    let p = ui.painter_at(view);

    if open {
        // ビューとの境の薄い影
        for k in 0..8 {
            let x = panel.left() - 1.0 - k as f32;
            let alpha = (46.0 - k as f32 * 5.5).max(0.0) as u8;
            w::fill(
                &p,
                Rect::from_min_max(pos2(x, panel.top()), pos2(x + 1.0, panel.bottom())),
                Color32::from_black_alpha(alpha),
            );
        }
        w::fill(&p, panel, t::PANEL_BG);
        w::vline(&p, panel.left(), panel.top(), panel.bottom(), t::BORDER);
        let title = Rect::from_min_size(panel.min, vec2(panel.width(), t::PANEL_HEADER_HEIGHT));
        w::panel_title(
            &p,
            title,
            ctx.pick("ハイド", "Hide"),
            Some("visibility_off"),
        );
        let body = Rect::from_min_max(pos2(panel.left(), title.bottom()), panel.max);
        drawer_body(ui, ctx, body, pn);
    }

    // つまみ（縦に細長い・左側が丸い）
    let response = ui.interact(
        handle,
        Id::new(("hideface5.handle", pn.key)),
        Sense::click(),
    );
    let bg = if response.hovered() {
        t::CONTROL_HOVER
    } else {
        t::PANEL_HEADER
    };
    let radius = egui::CornerRadius {
        nw: 6,
        sw: 6,
        ne: 0,
        se: 0,
    };
    p.rect_filled(handle, radius, bg);
    p.rect_stroke(
        handle,
        radius,
        Stroke::new(1.0, t::BORDER),
        egui::StrokeKind::Inside,
    );
    chevron(
        &p,
        handle,
        !open,
        if response.hovered() {
            Color32::WHITE
        } else {
            t::TEXT
        },
    );
    if response.on_hover_text(ctx.pick("ハイド", "Hide")).clicked() {
        ctx.values.set_bool(&open_key, !open);
    }
}

/// 引き出しの中身: 隠す・すべて表示のボタン、隠している物、ボーンで隠す、プリセット、ハイドの設定（あふれたらスクロール）。
fn drawer_body(ui: &mut Ui, ctx: &mut SketchCtx, body: Rect, pn: &Pane) {
    let scroll_key = format!("hideface5.scroll.{}", pn.key);
    let mut offset = ctx.values.float(&scroll_key, 0.0);
    let mut child = ui.new_child(
        UiBuilder::new()
            .id_salt(("hideface5.drawer.body", pn.key))
            .max_rect(body),
    );
    child.set_clip_rect(body.intersect(ui.clip_rect()));
    let ui = &mut child;
    if ui.rect_contains_pointer(body) {
        offset -= ui.input(|i| i.smooth_scroll_delta.y);
    }
    let mut rows = Rows::new(body, 4.0 - offset);
    // 低いビューでは、プリセットを閉じて始める
    let tall = body.height() >= 480.0;
    let list = entries(ctx, pn);
    let key = pn.key;

    uv_buttons(ui, ctx, &mut rows, !pn.selected_faces.is_empty());
    rows.space(2.0);
    if section(
        ui,
        ctx,
        &mut rows,
        key,
        "list",
        ctx.pick("隠している物", "Hidden"),
        "visibility_off",
        true,
    ) {
        entry_rows(ui, ctx, &mut rows, &list);
    }
    if section(
        ui,
        ctx,
        &mut rows,
        key,
        "bone",
        ctx.pick("ボーンで隠す", "Hide by Bone"),
        "accessibility",
        true,
    ) {
        bone_rows(ui, ctx, &mut rows, &pn.selected_bones);
    }
    if section(
        ui,
        ctx,
        &mut rows,
        key,
        "presets",
        ctx.pick("プリセット", "Presets"),
        "save",
        tall,
    ) {
        preset_rows(ui, ctx, &mut rows);
    }
    if section(
        ui,
        ctx,
        &mut rows,
        key,
        "settings",
        ctx.pick("ハイドの設定", "Hide Settings"),
        "tune",
        false,
    ) {
        settings_rows(ui, ctx, &mut rows);
    }
    let content = rows.used() + offset + 6.0;
    let scroll = Scroll::new(body, content, &mut offset);
    let _ = scroll.end(ui, ("hideface5.scrollbar", pn.key), &mut offset);
    ctx.values.set_float(&scroll_key, offset);
}

// ───────── 3D ビューの中の小さなシェルフ ─────────

const HIDE_PRESETS: [(&str, &str); 3] = [
    ("隠さない", "Show All"),
    ("素体のみ", "Body Only"),
    ("服を外す", "Cloth Off"),
];
const POSE_PRESETS: [(&str, &str); 3] = [
    ("初期ポーズ", "Rest Pose"),
    ("ポーズ 1", "Pose 1"),
    ("ポーズ 2", "Pose 2"),
];
/// プリセットの一覧の「プリセットを追加」の行の値（押すとハイドのモードへ移る）。
const ADD_PRESET: usize = usize::MAX;

const PILL_HEIGHT: f32 = 26.0;
const EYE_WIDTH: f32 = 30.0;
const ARROW_WIDTH: f32 = 22.0;
const BODY_WIDTH: f32 = 196.0;
const BODY_ROW: f32 = 26.0;

/// シェルフ 1 つ（ビューごとに 1 つ。左下に固定）。
pub struct Shelf {
    /// ビューごとに別の名前。
    pub key: &'static str,
    /// シェルフの左下（ビューの左下に固定）。
    pub anchor: Pos2,
    pub look: ShelfLook,
    /// このビューに隠すを当てているか（目の切り替えの初めの値）。
    pub applied: bool,
    /// いま当てているハイドのプリセットとポーズのプリセットの番号。
    pub hide: usize,
    pub pose: usize,
}

/// 閉じた形の地（キャンバスの右上の「UV」のボタンと同じ。当てていないときは暗い）。
fn pill_bg(dimmed: bool) -> Color32 {
    if dimmed {
        Color32::from_black_alpha(200)
    } else {
        Color32::from_black_alpha(120)
    }
}

/// シェルフを描く。閉じた形は「目・▽」だけ（文字なし）。目を押すとそのビューの隠すを当てる・当てないの切り替え、▽ を押すと、
/// 上へ開いてハイドとポーズのプリセットを選ぶドロップダウンが 2 つ出る。
pub fn shelf(ui: &mut Ui, ctx: &mut SketchCtx, s: &Shelf) {
    let lang = ctx.lang;
    let open_key = format!("hideface5.shelf.open.{}", s.key);
    let open = ctx.values.bool(&open_key, s.look != ShelfLook::Closed);
    let applied_key = format!("hideface5.shelf.applied.{}", s.key);
    let applied = ctx.values.bool(&applied_key, s.applied);
    let dimmed = !applied;

    let pill_w = EYE_WIDTH + ARROW_WIDTH;
    let pill = Rect::from_min_size(
        pos2(s.anchor.x, s.anchor.y - PILL_HEIGHT),
        vec2(pill_w, PILL_HEIGHT),
    );
    let eye = Rect::from_min_size(pill.min, vec2(EYE_WIDTH, PILL_HEIGHT));
    let arrow = Rect::from_min_max(pos2(eye.right(), pill.top()), pill.max);
    let p = ui.painter().clone();
    w::rounded(&p, pill, pill_bg(dimmed), 6.0);

    // 押せる範囲は 2 つ（目・▽）。乗せた・押した範囲だけ明るくする
    let eye_r = ui.interact(eye, Id::new(("hideface5.shelf.eye", s.key)), Sense::click());
    let arrow_r = ui.interact(
        arrow,
        Id::new(("hideface5.shelf.arrow", s.key)),
        Sense::click(),
    );
    let radius = |nw: u8, ne: u8| egui::CornerRadius {
        nw,
        sw: nw,
        ne,
        se: ne,
    };
    for (part, resp, corners, lit) in [
        (eye, &eye_r, radius(6, 0), false),
        (arrow, &arrow_r, radius(0, 6), open),
    ] {
        let bg = if lit {
            Some(t::ACCENT_DIM)
        } else if resp.is_pointer_button_down_on() {
            Some(Color32::from_black_alpha(220))
        } else if resp.hovered() {
            Some(Color32::from_black_alpha(120))
        } else {
            None
        };
        if let Some(bg) = bg {
            p.rect_filled(part, corners, bg);
        }
    }
    let fg = if dimmed { t::TEXT_DIM } else { t::TEXT };
    let hot = |r: &egui::Response| if r.hovered() { Color32::WHITE } else { fg };
    w::icon(
        &p,
        eye,
        if applied {
            "visibility"
        } else {
            "visibility_off"
        },
        hot(&eye_r),
        16.0,
    );
    w::vline(
        &p,
        arrow.left(),
        pill.top() + 5.0,
        pill.bottom() - 5.0,
        Color32::from_white_alpha(50),
    );
    w::icon(
        &p,
        arrow,
        "arrow_drop_down",
        if open || arrow_r.hovered() {
            Color32::WHITE
        } else {
            fg
        },
        18.0,
    );
    let eye_r = eye_r.on_hover_text(lang.pick("このビューで隠す", "Hide in This View"));
    let arrow_r = arrow_r.on_hover_text(lang.pick("プリセットを選ぶ", "Choose Presets"));
    if eye_r.clicked() {
        ctx.values.set_bool(&applied_key, !applied);
    }
    if arrow_r.clicked() {
        ctx.values.set_bool(&open_key, !open);
    }
    if !open {
        return;
    }

    // 閉じた形の上に開く: ハイドとポーズのプリセットを選ぶ 2 つのドロップダウン
    let body_h = 2.0 * (BODY_ROW + 4.0) + 4.0;
    let body = Rect::from_min_size(
        pos2(pill.left(), pill.top() - 4.0 - body_h),
        vec2(BODY_WIDTH, body_h),
    );
    w::rounded(&p, body, Color32::from_black_alpha(170), 6.0);
    w::outline(&p, body, t::BORDER, 1.0, 6.0);
    let rows = [
        (
            "visibility_off",
            &HIDE_PRESETS,
            s.hide,
            "hide",
            lang.pick("ハイドのプリセット", "Hide Preset"),
        ),
        (
            "accessibility",
            &POSE_PRESETS,
            s.pose,
            "pose",
            lang.pick("ポーズのプリセット", "Pose Preset"),
        ),
    ];
    for (k, (icon, list, current, name, tip)) in rows.into_iter().enumerate() {
        let y = body.top() + 4.0 + k as f32 * (BODY_ROW + 4.0);
        w::icon(
            &p,
            Rect::from_min_size(pos2(body.left() + 6.0, y), vec2(18.0, BODY_ROW)),
            icon,
            t::TEXT_DIM,
            15.0,
        );
        let dd = Rect::from_min_size(
            pos2(body.left() + 28.0, y + 1.0),
            vec2(body.width() - 34.0, BODY_ROW - 2.0),
        );
        let key = format!("hideface5.shelf.{name}.{}", s.key);
        let chosen_now = ctx.values.index(&key, current);
        let (response, boxr) = w::dropdown(
            ui,
            dd,
            ("hideface5.shelf.dd", s.key, k),
            None,
            ctx.pick(list[chosen_now].0, list[chosen_now].1),
            Some(tip),
            true,
            0.0,
        );
        let id = Id::new(("hideface5.shelf.menu", s.key, k));
        let mut state: Option<PopupState> = ui.data(|d| d.get_temp(id));
        let list_open = s.look == ShelfLook::OpenWithList && k == 0;
        if response.clicked() || (ctx.still && list_open && state.is_none()) {
            // 下に入りきらないときは、ドロップダウンの上へ開く（自分を隠さない）
            state = Some(PopupState::new(ui.ctx(), boxr).with_min_width(boxr.width()));
        }
        if let Some(mut menu_state) = state {
            let mut entries: Vec<MenuEntry<usize>> = list
                .iter()
                .enumerate()
                .map(|(i, (ja, en))| MenuEntry::item(ctx.pick(ja, en), i).radio(i == chosen_now))
                .collect();
            if k == 0 {
                // ハイドのプリセットの一覧の一番下: プリセットを追加（ハイドのモードへ移る）
                entries.push(MenuEntry::Separator);
                entries.push(
                    MenuEntry::item(ctx.pick("プリセットを追加", "Add Preset"), ADD_PRESET)
                        .icon("add", false),
                );
            }
            match menu::show(ui.ctx(), id, &mut menu_state, &entries, &[boxr]) {
                PopupOutcome::Chosen(i) => {
                    if i != ADD_PRESET {
                        ctx.values.set_index(&key, i);
                    }
                    ui.data_mut(|d| d.remove::<PopupState>(id));
                }
                PopupOutcome::Open => {
                    ui.data_mut(|d| d.insert_temp(id, menu_state));
                }
                _ => {
                    if ctx.still && list_open {
                        ui.data_mut(|d| d.insert_temp(id, menu_state));
                    } else {
                        ui.data_mut(|d| d.remove::<PopupState>(id));
                    }
                }
            }
        }
    }
}

// ───────── 引き出しの中身の部品 ─────────

fn open_key(pane: &str, id: &str) -> String {
    format!("hideface5.open.{pane}.{id}")
}

/// 大見出し（開閉を覚える）。開いているかを返す。
#[allow(clippy::too_many_arguments)]
fn section(
    ui: &mut Ui,
    ctx: &mut SketchCtx,
    rows: &mut Rows,
    pane: &str,
    id: &'static str,
    title: &str,
    icon: &str,
    default_open: bool,
) -> bool {
    let open = ctx.values.bool(&open_key(pane, id), default_open);
    rows.indent = 0.0;
    let header = rows.full_row(t::PANEL_HEADER_HEIGHT, 5.0);
    let out = w::section_header(
        ui,
        header,
        ("hideface5.section", pane, id),
        title,
        open,
        Some(icon),
        None,
    );
    if out.open != open {
        ctx.values.set_bool(&open_key(pane, id), out.open);
    }
    if out.open {
        rows.indent = t::SECTION_INDENT;
    }
    out.open
}

/// ボーンの影響で隠す（今のポーズの欄の「面を隠す」と同じ並び）。頭に、選んだボーンの名前を 1 行（複数なら「Hand.L ほか 2」）。
/// 選んだボーンがあるときだけ「選んだボーンの面を隠す」を押せる。
fn bone_rows(ui: &mut Ui, ctx: &SketchCtx, rows: &mut Rows, selected: &[usize]) {
    if let Some(first) = selected.first() {
        let row = rows.row(ROW, 2.0);
        let p = ui.painter().clone();
        w::icon(
            &p,
            Rect::from_min_size(row.min, vec2(16.0, row.height())),
            "accessibility",
            t::TEXT_DIM,
            13.0,
        );
        let name = if selected.len() > 1 {
            let more = selected.len() - 1;
            format!(
                "{}{}",
                BONES[*first].0,
                if ctx.lang == yolu_app::lang::Lang::Ja {
                    format!(" ほか {more}")
                } else {
                    format!(" +{more} more")
                }
            )
        } else {
            BONES[*first].0.to_owned()
        };
        w::text(
            &p,
            Rect::from_min_max(pos2(row.left() + 20.0, row.top()), row.max),
            &name,
            t::LABEL,
            Align::Left,
        );
    }
    let row = rows.row(ROW, 2.0);
    let _ = w::toggle(
        ui,
        row,
        "hideface5.children",
        ctx.pick("子を含める", "Include Children"),
        true,
        None,
        true,
    );
    let _ = percent_row(
        ui,
        rows,
        "hideface5.threshold",
        ctx.pick("しきい値", "Threshold"),
        0.5,
        (0.0, 1.0),
        None,
        true,
    );
    let row = rows.row(24.0, 4.0);
    let _ = w::button(
        ui,
        row,
        "hideface5.bone",
        ctx.pick("選んだボーンの面を隠す", "Hide Selected Bone"),
        false,
        !selected.is_empty(),
        None,
        Some("visibility_off"),
    );
}

/// 隠している物 1 行ずつ（アイコン・名前・状態・外す ×）。
fn entry_rows(ui: &mut Ui, ctx: &SketchCtx, rows: &mut Rows, list: &[Entry]) {
    let p = ui.painter().clone();
    for (i, e) in list.iter().enumerate() {
        let row = rows.row(ROW, 1.0);
        w::icon(
            &p,
            Rect::from_min_size(row.min, vec2(16.0, row.height())),
            e.icon,
            t::TEXT_DIM,
            13.0,
        );
        let button = Rect::from_min_size(
            pos2(row.right() - ICON_W, row.top()),
            vec2(ICON_W, row.height()),
        );
        let status_w = if e.status.is_empty() {
            0.0
        } else {
            w::text_width(&p, &e.status, t::LABEL_DIM)
        };
        w::text(
            &p,
            Rect::from_min_max(
                pos2(button.left() - status_w - 4.0, row.top()),
                pos2(button.left() - 2.0, row.bottom()),
            ),
            &e.status,
            t::LABEL_DIM,
            Align::Right,
        );
        let name_rect = Rect::from_min_max(
            pos2(row.left() + 20.0, row.top()),
            pos2(button.left() - status_w - 8.0, row.bottom()),
        );
        w::text(
            &p,
            name_rect,
            &w::fit(&p, &e.name, name_rect.width(), t::LABEL),
            t::LABEL,
            Align::Left,
        );
        let _ = w::icon_button(
            ui,
            button,
            ("hideface5.remove", i),
            "close",
            ctx.pick("この項目を外す", "Remove this entry"),
            false,
            true,
            12.0,
        );
    }
}

/// 隠し方（プリセット）: 今の隠し方を名前をつけて保存、入れる・外す、名前の変更、削除。
fn preset_rows(ui: &mut Ui, ctx: &SketchCtx, rows: &mut Rows) {
    rows.space(4.0);
    let row = rows.row(ROW, 4.0);
    let save_w = 74.0;
    let field = Rect::from_min_max(row.min, pos2(row.right() - save_w - 4.0, row.bottom()));
    let _ = w::text_field(
        ui,
        field,
        "hideface5.name",
        ctx.pick("隠し方", "Hide Set"),
        None,
        false,
    );
    let save = Rect::from_min_max(pos2(row.right() - save_w, row.top()), row.max);
    let _ = w::button(
        ui,
        save,
        "hideface5.save",
        ctx.pick("保存", "Save"),
        false,
        true,
        None,
        Some("save"),
    );
    let presets = [
        (ctx.pick("素体のみ", "Body Only"), true),
        (ctx.pick("服を外す", "Cloth Off"), false),
    ];
    let p = ui.painter().clone();
    for (i, (name, on)) in presets.into_iter().enumerate() {
        let row = rows.row(ROW, 1.0);
        let delete = Rect::from_min_size(
            pos2(row.right() - ICON_W, row.top()),
            vec2(ICON_W, row.height()),
        );
        let rename = delete.translate(vec2(-ICON_W, 0.0));
        let label = Rect::from_min_max(row.min, pos2(rename.left() - 4.0, row.bottom()));
        let shown = w::fit(&p, name, label.width() - 24.0, t::LABEL);
        let _ = w::toggle(ui, label, ("hideface5.preset", i), &shown, on, None, true);
        let _ = w::icon_button(
            ui,
            rename,
            ("hideface5.rename", i),
            "edit",
            ctx.pick("名前を変える", "Rename"),
            false,
            true,
            13.0,
        );
        let _ = w::icon_button(
            ui,
            delete,
            ("hideface5.delete", i),
            "delete",
            ctx.pick("この隠し方を消す", "Delete this hide set"),
            false,
            true,
            13.0,
        );
    }
}

/// 選んだ面を隠す・すべて表示（名前とキー。選んだ面が無いときは隠すを押せない）。
fn uv_buttons(ui: &mut Ui, ctx: &SketchCtx, rows: &mut Rows, can_hide: bool) {
    let row = rows.row(24.0, 4.0);
    let _ = w::button(
        ui,
        row,
        "hideface5.hide",
        ctx.pick("選んだ面を隠す（H）", "Hide Selected Faces (H)"),
        false,
        can_hide,
        None,
        Some("visibility_off"),
    );
    let row = rows.row(24.0, 4.0);
    let _ = w::button(
        ui,
        row,
        "hideface5.show",
        ctx.pick("すべて表示（Alt+H）", "Show All (Alt+H)"),
        false,
        true,
        None,
        Some("visibility"),
    );
}

/// ハイドのモードで使う設定。
fn settings_rows(ui: &mut Ui, ctx: &SketchCtx, rows: &mut Rows) {
    let row = rows.row(ROW, 2.0);
    let _ = w::toggle(
        ui,
        row,
        "hideface5.outline",
        ctx.pick("隠した面の輪郭を出す", "Show Outline of Hidden Faces"),
        true,
        None,
        true,
    );
    let row = rows.row(ROW, 2.0);
    let _ = w::toggle(
        ui,
        row,
        "hideface5.keep",
        ctx.pick("隠したあとも選びを残す", "Keep Selection After Hiding"),
        false,
        None,
        true,
    );
}
