//! ブラシの詳細の欄。詳細のウィンドウ（`brush_detail`）が、左の区分（`brushes::rows::Category`）ごとに、その区分の行（`brush_rows`）を縦に積む。
//! 区分の中身は区分の表（`brushes::rows::Row`）が決め、ここは区分ごとの行以外の部品（筆先の見本と一覧・入り抜きの影響先）と、区分ごとの「既定に戻す」。
//! 値は全部入りのブラシ（`AppState::m2.brush`）と基本の値（`AppState::brush`）を直に変え、ストロークを始めたときに写して固定する
//! （途中で変えても、そのストロークには効かない）。3D のビューの面のダブも同じ欄を 2D と同じ式で使う（長さの欄は、3D では画面の点）。

use std::collections::HashMap;

use egui::{
    pos2, vec2, Color32, ColorImage, Id, Rect, Sense, TextureHandle, TextureId, TextureOptions, Ui,
};

use super::brush_rows::{self, hardness_applies as rows_hardness_applies};
use super::properties::group_label;
use super::tip_library;
use crate::brushes::rows::{Category, Dynamics, Row};
use crate::engine::{
    BrushEffect, ColorDynamics, ColorMix, Controls, Influences, Jitter, TaperTargets, TipShape,
};
use crate::lang::Lang;
use crate::m2::{self, tip_label, BrushOp, UiOp};
use crate::state::{Action, AppState, BrushState};
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, NumberFormat, Rows, SliderSpec};
use yolu_core::brush::Item;

/// 硬さが効くか（画像の筆先では効かない。ツールプロパティと欄が同じ判定を使う）。
pub fn hardness_applies(app: &AppState) -> bool {
    rows_hardness_applies(app)
}

/// 区分の欄を縦に積む（見出しはここに無い。ウィンドウが出す）。見出しの区分（ブラシ形状・2-ブラシ形状）には行が無い。
pub fn category_body(
    ui: &mut Ui,
    app: &mut AppState,
    rows: &mut Rows,
    ctx: &egui::Context,
    category: Category,
) {
    let lang = app.lang;
    rows.indent = 0.0;
    if category == Category::Tip {
        tip_header(ui, app, rows, ctx, lang);
    }
    let mut projection_labelled = false;
    for row in Row::of(category) {
        if !brush_rows::visible(app, row) {
            continue;
        }
        // 3D の塗りの切り替えは、3D のビューが出ているときだけ。小さな見出しの下に
        if matches!(row, Row::ProjectionHidden) && !projection_labelled {
            projection_labelled = true;
            rows.space(5.0);
            group_label(ui, rows, "3D");
        }
        brush_rows::detail_row(ui, app, ctx, row, rows);
    }
    match category {
        Category::Tip => tip_lists(ui, app, rows, ctx, lang),
        Category::Taper => taper_targets(ui, app, rows, lang),
        _ => {}
    }
}

/// 項目の影響元を既定へ戻す（大きさ・不透明度・流量の筆圧の切り替えは `AppState::brush` へも）。
fn reset_item(app: &mut AppState, item: Item) {
    let default = Influences::default();
    brush_rows::set_influence(app, Dynamics::Item(item), default.get(item).clone());
}

/// 筆先の形を既定に戻すとき、「ストローク」の区分の行（先端同士の比較暗・連続吹き付け・リボン）の値だけを残した筆先の形。
fn stroke_rows_only(tip: &TipShape) -> TipShape {
    TipShape {
        darken_overlap: tip.darken_overlap,
        continuous: tip.continuous,
        ribbon: tip.ribbon,
        ..TipShape::default()
    }
}

/// 区分の設定（行の値と、行の影響元）を既定に戻す（見出しの右の「既定に戻す」）。目の印（どの行をツールプロパティに出すか）は変えない。
pub fn reset_category(app: &mut AppState, category: Category) {
    let defaults = BrushState::default();
    match category {
        Category::Size => {
            app.brush.radius = defaults.radius;
            let brush = &mut app.m2.brush;
            brush.controls.size_by_screen = false;
            brush.controls.min_one_pixel = false;
            brush.jitter.size = Jitter::default().size;
            reset_item(app, Item::Size);
        }
        Category::Ink => {
            app.brush.opacity = defaults.opacity;
            app.brush.flow = defaults.flow;
            let brush = &mut app.m2.brush;
            brush.jitter.opacity = 0.0;
            brush.jitter.flow = 0.0;
            brush.mix = ColorMix::default();
            brush.effect = BrushEffect::Paint;
            for item in [Item::Opacity, Item::Flow, Item::MixPaint, Item::MixDensity] {
                reset_item(app, item);
            }
        }
        Category::ColorChange => {
            let brush = &mut app.m2.brush;
            brush.color = ColorDynamics::default();
            // 色の項目の影響元も一緒に戻す（値だけを戻すと、影響元の入力が残って次に入れたとき揺れ方が変わる）
            for item in yolu_core::ColorItem::ALL {
                *brush.influences.color_mut(item) = Default::default();
            }
            app.m2.color_tip_stash = Default::default();
            app.m2.color_stroke_stash = Default::default();
        }
        Category::AntiAlias => app.brush.anti_alias = defaults.anti_alias,
        Category::Tip => {
            app.brush.hardness = defaults.hardness;
            let brush = &mut app.m2.brush;
            brush.tip = stroke_rows_only(&brush.tip);
            brush.controls.tilt_angle = false;
            brush.controls.rotation_angle = false;
            brush.jitter.angle = 0.0;
            brush.jitter.roundness = 0.0;
            for item in [Item::Hardness, Item::Roundness] {
                reset_item(app, item);
            }
        }
        Category::Spray => {
            let j = &mut app.m2.brush.jitter;
            (j.scatter, j.count) = (0.0, 1);
        }
        Category::Stroke => {
            app.brush.spacing = defaults.spacing;
            app.m2.brush.tip.darken_overlap = false;
            app.m2.brush.tip.continuous = false;
            app.m2.brush.tip.ribbon = false;
            app.m2.brush.influences.speed_max = Influences::DEFAULT_SPEED_MAX;
            app.view3d.projection = Default::default();
        }
        Category::Paper => {
            app.m2.brush.texture = None;
            app.m2.texture_prefs = m2::TexturePrefs::default();
            reset_item(app, Item::TextureDensity);
        }
        Category::DualTip => {
            app.m2.brush.dual = None;
            app.m2.dual_stash = Default::default();
        }
        Category::DualSpray => {
            let d = yolu_core::DualBrush::default();
            for dual in [app.m2.brush.dual.as_mut(), Some(&mut app.m2.dual_stash)]
                .into_iter()
                .flatten()
            {
                (dual.scatter, dual.count) = (d.scatter, d.count);
            }
        }
        Category::DualStroke => {
            let d = yolu_core::DualBrush::default();
            for dual in [app.m2.brush.dual.as_mut(), Some(&mut app.m2.dual_stash)]
                .into_iter()
                .flatten()
            {
                dual.spacing = d.spacing;
            }
        }
        Category::DualPaper => {
            for dual in [app.m2.brush.dual.as_mut(), Some(&mut app.m2.dual_stash)]
                .into_iter()
                .flatten()
            {
                dual.texture = None;
            }
        }
        Category::Erase => app.m2.erase_mode = false,
        Category::WaterEdge => app.m2.brush.water_edge = None,
        Category::ReferenceLines => app.m2.brush.reference_lines = Default::default(),
        Category::Stabilization => {
            let a = &mut app.m2.brush.assist;
            (a.stabilizer, a.curve) = (0.0, false);
            a.stabilizer_speed = None;
            a.sweep = 0.0;
            a.post = yolu_core::PostCorrection::default();
        }
        Category::Taper => {
            let brush = &mut app.m2.brush;
            brush.assist.taper_in = 0.0;
            brush.assist.taper_out = 0.0;
            brush.assist.taper_targets = TaperTargets::default();
            let c = Controls::default();
            brush.controls.fade_size = c.fade_size;
            brush.controls.fade_opacity = c.fade_opacity;
            brush.controls.fade_flow = c.fade_flow;
        }
        Category::Shape | Category::Dual => {}
    }
}

// ───────── 入り抜きの影響先 ─────────

/// 入り抜きを掛けられる項目の行（名前を行から引く）。
fn target_row(item: Item) -> Row {
    match item {
        Item::Size => Row::Size,
        Item::Opacity => Row::Opacity,
        Item::Flow => Row::Flow,
        Item::Hardness => Row::Hardness,
        Item::Roundness => Row::Roundness,
        Item::TextureDensity => Row::PaperDepth,
        Item::MixPaint => Row::MixPaint,
        Item::MixDensity => Row::MixDensity,
    }
}

/// 入り抜きの影響先: 項目ごとに、掛けるかのチェックと最小値（掛けない項目の最小値は薄い）。
fn taper_targets(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, lang: Lang) {
    rows.space(6.0);
    group_label(ui, rows, lang.t("brushrows-taper-targets"));
    for item in Item::ALL {
        let row = target_row(item);
        let line = rows.row(brush_rows::ROW, brush_rows::GAP);
        let halves = Rows::split(line, 2, 6.0);
        let current = app.m2.brush.assist.taper_targets.min(item);
        let on = current.is_some();
        let next = w::toggle(
            ui,
            halves[0],
            ("brush.taper.on", item.id()),
            row.name(lang),
            on,
            None,
            true,
        );
        let mut min = current;
        if next != on {
            min = next.then_some(0.0);
        }
        let spec = SliderSpec::new(
            lang.t("brushrows-minimum"),
            0.0,
            100.0,
            NumberFormat::int("%"),
        )
        .enabled(next);
        let out = w::slider(
            ui,
            Rect::from_min_max(
                pos2(halves[1].left(), halves[1].top() + 1.0),
                pos2(halves[1].right(), halves[1].bottom() - 1.0),
            ),
            ("brush.taper.min", item.id()),
            (current.unwrap_or(0.0) * 100.0) as f32,
            &spec,
        );
        if out.changed && next {
            min = Some((out.value as f64 / 100.0).clamp(0.0, 1.0));
        }
        if min != current {
            if let Ok(targets) = app.m2.brush.assist.taper_targets.with(item, min) {
                app.m2.brush.assist.taper_targets = targets;
            }
        }
    }
}

// ───────── 形状（筆先） ─────────

/// 一覧の 1 マス（見本の大きさは `brushes::krita::THUMB`）。
const CELL: f32 = 40.0;
const CELL_GAP: f32 = 4.0;

/// 筆先の画像の見本（白地に黒。丸い筆先は縁がやわらかい円）。
fn tip_image(id: Option<&str>) -> ColorImage {
    crate::brushes::krita::thumbnail(id.and_then(yolu_core::builtin_tip).as_deref())
}

#[derive(Clone, Default)]
struct TipThumbs(HashMap<&'static str, TextureHandle>);

/// 筆先の見本の絵（初めて出すときに作って、文脈に覚えておく）。None は丸。
fn tip_texture(ctx: &egui::Context, id: Option<&'static str>) -> TextureId {
    let cache_id = Id::new("yolu.tip-thumbs");
    let mut cache: TipThumbs = ctx.data(|d| d.get_temp(cache_id)).unwrap_or_default();
    let key = id.unwrap_or("round");
    if !cache.0.contains_key(key) {
        let handle = ctx.load_texture(format!("tip-{key}"), tip_image(id), TextureOptions::LINEAR);
        cache.0.insert(key, handle);
        ctx.data_mut(|d| d.insert_temp(cache_id, cache.clone()));
    }
    cache.0[key].id()
}

fn tip_cell(ui: &mut Ui, r: Rect, id: Option<&'static str>, selected: bool, tooltip: &str) -> bool {
    let key = id.unwrap_or("round");
    let response = ui.interact(r, ui.make_persistent_id(("alpha.tip", key)), Sense::click());
    let texture = tip_texture(ui.ctx(), id);
    let p = ui.painter();
    w::rounded(p, r, Color32::WHITE, 3.0);
    p.image(
        texture,
        r.shrink(2.0),
        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
        Color32::WHITE,
    );
    if selected {
        w::outline(p, r.expand(2.0), t::ACCENT, 2.0, 4.0);
    } else if response.hovered() {
        w::outline(p, r.expand(1.0), t::ACCENT_DIM, 1.0, 4.0);
    }
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, tooltip)
    });
    let clicked = response.clicked();
    let _ = response.on_hover_text(tooltip);
    clicked
}

// ───────── ブラシ先端: 見本と一覧 ─────────

/// 今の筆先の見本と名前（ブラシ先端の区分の初め）。
fn tip_header(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, ctx: &egui::Context, lang: Lang) {
    let kind = tip_library::current(&app.m2.brush.tip);
    let head = rows.row(48.0, 6.0);
    let swatch = Rect::from_min_size(head.min, vec2(48.0, 48.0));
    let swatch_texture = match &kind {
        tip_library::Current::Round => Some(tip_texture(ctx, None)),
        tip_library::Current::Builtin(id) => Some(tip_texture(ctx, Some(id))),
        tip_library::Current::Krita(index) => app.brushes.krita.texture(ctx, *index),
        tip_library::Current::Image(tip) => Some(tip_library::image_texture(ctx, tip)),
    };
    let p = ui.painter();
    w::rounded(p, swatch, Color32::WHITE, 3.0);
    if let Some(texture) = swatch_texture {
        p.image(
            texture,
            swatch.shrink(2.0),
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE,
        );
    }
    let name: String = match &kind {
        tip_library::Current::Round => lang.t("panels-round-hardness-tip").to_owned(),
        tip_library::Current::Builtin(id) => tip_label(lang, id).to_owned(),
        tip_library::Current::Krita(index) => app
            .brushes
            .krita
            .name(*index)
            .map(str::to_owned)
            .unwrap_or_else(|| "Krita".to_owned()),
        tip_library::Current::Image(tip) => m2::tip_display(lang, tip),
    };
    let from = match &kind {
        tip_library::Current::Round => lang.t("panels-tip-tip"),
        tip_library::Current::Builtin(_) => lang.t("panels-built-in-image-tip"),
        tip_library::Current::Krita(_) => "Krita",
        tip_library::Current::Image(_) => lang.t("panels-imported-image-tip"),
    };
    let tx = head.left() + 56.0;
    let tw = head.width() - 56.0;
    let shown = w::fit(ui.painter(), &name, tw, t::LABEL_BOLD);
    w::text(
        ui.painter(),
        Rect::from_min_size(pos2(tx, head.top() + 6.0), vec2(tw, 18.0)),
        &shown,
        t::LABEL_BOLD,
        w::Align::Left,
    );
    let shown = w::fit(ui.painter(), from, tw, t::LABEL_DIM);
    w::text(
        ui.painter(),
        Rect::from_min_size(pos2(tx, head.top() + 26.0), vec2(tw, 16.0)),
        &shown,
        t::LABEL_DIM,
        w::Align::Left,
    );
}

/// 筆先の一覧（丸と組み込みの画像、同梱の Krita の筆先。ブラシ先端の区分の終わり）。
fn tip_lists(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, ctx: &egui::Context, lang: Lang) {
    let kind = tip_library::current(&app.m2.brush.tip);
    rows.space(5.0);
    group_label(ui, rows, lang.t("panels-built-in-tip"));
    let ids: Vec<Option<&'static str>> = std::iter::once(None)
        .chain(yolu_core::brush::BUILTIN_TIPS.into_iter().map(Some))
        .collect();
    let columns = (((rows.width() + CELL_GAP) / (CELL + CELL_GAP)).floor() as usize).max(1);
    for chunk in ids.chunks(columns) {
        let line = rows.row(CELL, CELL_GAP);
        for (k, id) in chunk.iter().enumerate() {
            let cell = Rect::from_min_size(
                pos2(line.left() + k as f32 * (CELL + CELL_GAP), line.top()),
                vec2(CELL, CELL),
            );
            let tip_name = id
                .map(|i| tip_label(lang, i))
                .unwrap_or(lang.t("panels-round-hardness-tip"));
            // 印は種類から決める（Krita・取り込んだ画像のときは丸にも組み込みにも付けない）
            let selected = match &kind {
                tip_library::Current::Round => id.is_none(),
                tip_library::Current::Builtin(b) => *id == Some(*b),
                _ => false,
            };
            if tip_cell(ui, cell, *id, selected, tip_name) {
                app.apply(Action::M2Ui(UiOp::Brush(BrushOp::Tip(*id))));
            }
        }
    }
    // 同梱の Krita の筆先
    tip_library::krita_section(ui, app, rows, ctx);
}

/// ステンシルのタブ（中身は `stencil_props`）。
pub fn stencil_tab(ui: &mut Ui, app: &mut AppState, rows: &mut Rows) {
    let ctx = ui.ctx().clone();
    super::stencil_props::stencil_tab(ui, app, rows, &ctx);
}
