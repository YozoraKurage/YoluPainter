//! 範囲の道具（バケツ・ポリゴン塗りつぶし・ID の色で選択）の欄: オプションバー（範囲・塗る/消す・不透明度・許容）と、プロパティの欄
//! （範囲の節と、続けてブラシのマテリアル。ID の色で選択は ID マップの節）。値は `AppState::region` で、操作は `Action::Region` を通す
//! （キー・試験と同じ道）。画面には名前と値だけを出し、説明はツールチップ。

use egui::{pos2, vec2, Rect, Ui};

use super::properties::{choice_row, group_label, open_popup, slider_row, status_row, toggle_row};
use crate::lang::Lang;
use crate::m2_menu::Popup;
use crate::region::idcolor::{hex_of, manual_state_lines, parse_rgb};
use crate::engine::SelectionCombine;
use crate::region::{kind_name, IdColorOp, RegionAction};
use crate::selection::{combine_name, combine_tooltip, SelAction, SelUiOp};
use crate::state::{Action, AppState, Tool};
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, NumberFormat, Rows, SliderSpec};

/// 範囲の欄の名前（オプションバーとプロパティで同じ）。
fn range_label(lang: Lang) -> &'static str {
    lang.pick("範囲", "Region")
}

/// 今の範囲の名前（バケツは近い色も）。
pub fn range_value(app: &AppState) -> &'static str {
    let lang = app.lang;
    if app.tool == Tool::Fill && app.region.by_color {
        lang.pick("近い色", "Similar colors")
    } else {
        kind_name(lang, app.region.kind)
    }
}

const RANGE_TIP_JA: &str = "クリック・ドラッグで塗る範囲: 三角形、つながったメッシュの塊、UV アイランド、このテクスチャセットのマテリアル全体";
const RANGE_TIP_EN: &str = "What a click or drag fills: the triangle, the connected mesh part, the UV island or the whole material of this texture set";

// ───────── オプションバー ─────────

struct Cursor {
    x: f32,
    y: f32,
    h: f32,
}

impl Cursor {
    fn next(&mut self, width: f32) -> Rect {
        let r = Rect::from_min_size(pos2(self.x + 4.0, self.y), vec2(width, self.h));
        self.x += width + 8.0;
        r
    }
}

fn opacity_slider(ui: &mut Ui, app: &mut AppState, at: Rect) {
    let lang = app.lang;
    let out = w::slider(
        ui,
        at,
        "options.opacity",
        app.brush.opacity * 100.0,
        &SliderSpec::new(lang.pick("不透明度", "Opacity"), 0.0, 100.0, NumberFormat::int("%")),
    );
    if out.changed {
        app.brush.opacity = out.value / 100.0;
    }
}

/// 塗る・消すの 1 組のボタン（マスクでは白・黒）。
fn paint_erase(ui: &mut Ui, app: &mut AppState, cursor: &mut Cursor) {
    let lang = app.lang;
    let mask = app.m2.edit_mask;
    let (paint, erase) = if mask {
        (lang.pick("白（見せる）", "White (show)"), lang.pick("黒（隠す）", "Black (hide)"))
    } else {
        (lang.pick("塗る", "Paint"), lang.pick("消す", "Erase"))
    };
    let (paint_tip, erase_tip) = if mask {
        (
            lang.pick("マスクを白で塗る（レイヤーを見せる）", "Fill the mask with white (shows the layer)"),
            lang.pick("マスクを黒で塗る（レイヤーを隠す）", "Fill the mask with black (hides the layer)"),
        )
    } else {
        (
            lang.pick("描画色と不透明度で塗る", "Fill with the paint color and the opacity"),
            lang.pick("透明にする", "Erase to transparent"),
        )
    };
    let width = |p: &egui::Painter, s: &str| w::text_width(p, s, t::LABEL) + 20.0;
    let pw = width(ui.painter(), paint);
    let ew = width(ui.painter(), erase);
    let erasing = app.region.erase;
    let a = cursor.next(pw);
    if w::button(ui, a, "region.paint", paint, !erasing, true, Some(paint_tip), None).clicked() {
        app.apply(Action::Region(RegionAction::Erase(false)));
    }
    cursor.x -= 6.0; // 塗る・消すは 1 組
    let b = cursor.next(ew);
    if w::button(ui, b, "region.erase", erase, erasing, true, Some(erase_tip), None).clicked() {
        app.apply(Action::Region(RegionAction::Erase(true)));
    }
}

fn tolerance_slider(ui: &mut Ui, app: &mut AppState, at: Rect, id: &str, max: f32, tip: &str, value: f32) -> Option<f32> {
    let lang = app.lang;
    let out = w::slider(
        ui,
        at,
        id,
        value,
        &SliderSpec::new(lang.pick("許容", "Tolerance"), 0.0, max, NumberFormat::int("")).tooltip(tip),
    );
    out.changed.then(|| out.value.round().clamp(0.0, max))
}

/// 選択範囲の組み合わせ方（置き換え・足す・引く・重ねる）。選択の道具のオプションバーと同じ値（`AppState::sel`）を切り替える。
/// 入りきらないぶんは出さない（`right` は使える右端）。
fn combine_buttons(ui: &mut Ui, app: &mut AppState, cursor: &mut Cursor, right: f32) {
    let lang = app.lang;
    for mode in [
        SelectionCombine::Replace,
        SelectionCombine::Add,
        SelectionCombine::Subtract,
        SelectionCombine::Intersect,
    ] {
        let name = combine_name(lang, mode);
        let width = w::text_width(ui.painter(), name, t::LABEL) + 22.0;
        if cursor.x + 4.0 + width > right {
            return;
        }
        let at = cursor.next(width);
        cursor.x -= 4.0;
        if w::button(
            ui,
            at,
            ("options.id-combine", mode),
            name,
            app.sel.combine == mode,
            true,
            Some(combine_tooltip(lang, mode)),
            None,
        )
        .clicked()
        {
            app.apply(Action::Sel(SelAction::Ui(SelUiOp::Combine(mode))));
        }
    }
    cursor.x += 8.0;
}

/// オプションバーの中身（ツールのアイコンの右から）。
pub fn options(ui: &mut Ui, app: &mut AppState, r: Rect, x: f32) {
    let ctx = ui.ctx().clone();
    let lang = app.lang;
    let mut cursor = Cursor { x, y: r.top() + 6.0, h: r.height() - 12.0 };
    match app.tool {
        Tool::Fill | Tool::PolygonFill => {
            let at = cursor.next(190.0);
            let tip = lang.pick(RANGE_TIP_JA, RANGE_TIP_EN);
            let (response, b) = w::dropdown(ui, at, "options.region", Some(range_label(lang)), range_value(app), Some(tip), true, 0.0);
            if response.clicked() {
                open_popup(app, &ctx, Popup::Region, b, b.width());
            }
            paint_erase(ui, app, &mut cursor);
            let at = cursor.next(130.0);
            opacity_slider(ui, app, at);
            if app.tool == Tool::Fill && app.region.by_color {
                let at = cursor.next(150.0);
                let v = app.region.tolerance as f32;
                if let Some(v) = tolerance_slider(ui, app, at, "options.tolerance", 255.0, lang.pick("押した画素の色との各成分の差の上限", "The largest per-channel difference from the pressed pixel"), v) {
                    app.apply(Action::Region(RegionAction::Tolerance(v as u8)));
                }
                let at = cursor.next(100.0);
                let v = w::toggle(ui, at, "options.contiguous", lang.pick("隣接", "Contiguous"), app.region.contiguous, None, true);
                if v != app.region.contiguous {
                    app.apply(Action::Region(RegionAction::Contiguous(v)));
                }
                let at = cursor.next(150.0);
                let v = w::toggle(
                    ui,
                    at,
                    "options.sample-all",
                    lang.pick("全レイヤーを見る", "Sample All Layers"),
                    app.region.sample_all,
                    Some(lang.pick("選んだレイヤーでなく合成を見る", "Use the composite instead of the selected layer")),
                    true,
                );
                if v != app.region.sample_all {
                    app.apply(Action::Region(RegionAction::SampleAll(v)));
                }
            }
        }
        Tool::IdSelect => {
            combine_buttons(ui, app, &mut cursor, r.right() - 12.0);
            let at = cursor.next(170.0);
            let v = app.region.id_tolerance as f32;
            if let Some(v) = tolerance_slider(
                ui,
                app,
                at,
                "options.id-tolerance",
                255.0,
                lang.pick(
                    "画素の ID の色が、クリックした色からどこまで離れていてよいか（8 bit のチャンネルの差の最大）。焼いた ID の色は、部品が 4080 個までなら互いに 17 以上離れています",
                    "How far (largest 8-bit channel difference) a pixel's ID color may be from the clicked one. Baked ID colors of up to 4080 parts differ by 17 or more",
                ),
                v,
            ) {
                app.apply(Action::Region(RegionAction::IdTolerance(v as u8)));
            }
            if let Err(reason) = app.usable_id_map() {
                let shown = w::fit(ui.painter(), &reason, (r.right() - cursor.x - 12.0).max(40.0), t::LABEL_DIM);
                let at = cursor.next(w::text_width(ui.painter(), &shown, t::LABEL_DIM) + 6.0);
                w::text(ui.painter(), at, &shown, t::LABEL_DIM.with_color(t::WARNING), w::Align::Left);
            }
        }
        _ => {}
    }
}

// ───────── プロパティの欄 ─────────

/// 範囲の道具の文脈のとき、欄に出すか（ID の色で選択はいつも。バケツとポリゴン塗りつぶしは、ペイントの層かマスクに塗るあいだ）。
pub fn owns_properties(app: &AppState, paint_context: bool) -> bool {
    match app.tool {
        Tool::IdSelect => true,
        Tool::Fill | Tool::PolygonFill => paint_context,
        _ => false,
    }
}

pub fn body(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, ctx: &egui::Context) {
    match app.tool {
        Tool::IdSelect => id_section(ui, app, rows),
        Tool::Fill | Tool::PolygonFill => {
            region_section(ui, app, rows, ctx);
            super::material::material_section(ui, app, rows);
        }
        _ => {}
    }
}

fn region_section(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, ctx: &egui::Context) {
    let lang = app.lang;
    let (open, _) = super::properties::section(
        ui,
        app,
        rows,
        "region",
        lang.pick("範囲", "Region"),
        "view_in_ar",
        None,
    );
    if !open {
        return;
    }
    let value = range_value(app);
    // 見出しが「範囲」なので、箱は名前なしで幅いっぱいに（狭い欄でも値を切らない）
    if let Some(b) = choice_row(ui, rows, "region.kind", "", value, Some(lang.pick(RANGE_TIP_JA, RANGE_TIP_EN)), true) {
        open_popup(app, ctx, Popup::Region, b, b.width());
    }
    if app.tool == Tool::Fill && app.region.by_color {
        let tol = app.region.tolerance as f32;
        if let Some(v) = slider_row(
            ui,
            rows,
            "region.tolerance",
            lang.pick("許容", "Tolerance"),
            tol,
            (0.0, 255.0),
            NumberFormat::int(""),
            Some(lang.pick("押した画素の色との各成分の差の上限", "The largest per-channel difference from the pressed pixel")),
            true,
        ) {
            app.apply(Action::Region(RegionAction::Tolerance(v.round() as u8)));
        }
        let c = app.region.contiguous;
        if let Some(v) = toggle_row(ui, rows, "region.contiguous", lang.pick("隣接", "Contiguous"), c, None, true) {
            app.apply(Action::Region(RegionAction::Contiguous(v)));
        }
        let s = app.region.sample_all;
        if let Some(v) = toggle_row(
            ui,
            rows,
            "region.sample-all",
            lang.pick("全レイヤーを見る", "Sample All Layers"),
            s,
            Some(lang.pick("選んだレイヤーでなく合成を見る", "Use the composite instead of the selected layer")),
            true,
        ) {
            app.apply(Action::Region(RegionAction::SampleAll(v)));
        }
    } else if app.region_model().is_none() {
        status_row(ui, rows, &app.region_missing_reason());
    }
    rows.space(4.0);
}

fn id_section(ui: &mut Ui, app: &mut AppState, rows: &mut Rows) {
    let lang = app.lang;
    let (open, _) = super::properties::section(
        ui,
        app,
        rows,
        "id-map",
        lang.pick("ID マップ", "ID Map"),
        "palette",
        None,
    );
    if !open {
        return;
    }
    let usable = app.usable_id_map();
    match &usable {
        Ok(_) => status_row(ui, rows, lang.pick("焼いた ID マップを使えます", "A baked ID map is ready")),
        Err(reason) => status_row(ui, rows, reason),
    }
    // ベイクの窓を、ID マップにチェックを入れて開く
    let row = rows.row(24.0, 4.0);
    let label = if usable.is_ok() {
        lang.pick("ID マップをベイクし直す…", "Bake ID Map Again…")
    } else {
        lang.pick("ID マップをベイク…", "Bake ID Map…")
    };
    let baking = app.bake.is_baking();
    if w::button(ui, row, "id.bake", label, usable.is_err(), !baking && !app.is_stroking(), None, None).clicked() {
        app.apply(Action::Bake(crate::bake::BakeAction::Map(yolu_core::mesh_maps::MeshMapKind::Id, true)));
        app.apply(Action::Bake(crate::bake::BakeAction::OpenWindow));
    }
    let tol = app.region.id_tolerance as f32;
    if let Some(v) = slider_row(
        ui,
        rows,
        "id.tolerance",
        lang.pick("許容", "Tolerance"),
        tol,
        (0.0, 255.0),
        NumberFormat::int(""),
        Some(lang.pick(
            "画素の ID の色が、クリックした色からどこまで離れていてよいか（8 bit のチャンネルの差の最大）",
            "How far (largest 8-bit channel difference) a pixel's ID color may be from the clicked one",
        )),
        true,
    ) {
        app.apply(Action::Region(RegionAction::IdTolerance(v.round() as u8)));
    }
    manual_colors(ui, app, rows);
    rows.space(4.0);
}

/// 部品の切り替えの表示（今のセットの部品の一覧の中の位置 / 個数。モデル全体の部品の番号ではない）。
pub fn part_position(lang: crate::lang::Lang, at: usize, count: usize) -> String {
    lang.pick(
        format!("部品 {} / {count}", at + 1),
        format!("Part {} / {count}", at + 1),
    )
}

/// 手動の ID の色（メッシュの塊ごと。文書の状態で、.ylp にはまだ書けない）。
fn manual_colors(ui: &mut Ui, app: &mut AppState, rows: &mut Rows) {
    let lang = app.lang;
    group_label(ui, rows, lang.pick("部品の手動の色", "Manual part colors"));
    let count = app.doc.id_colors().colors().len();
    let free = !app.is_stroking();
    if app.id_colors_foreign() {
        status_row(
            ui,
            rows,
            lang.pick("別のモデルの手動の色です", "These manual colors belong to another model"),
        );
    } else {
        // モデルが替わった直後は、部品を別のスレッドで求め終えるまで一覧を出さない（UI を止めて待たない）
        let found = app.id_set_parts();
        if let Some(parts) = found.as_deref().filter(|p| !p.is_empty()) {
            let at = app.region.id.part.min(parts.len() - 1);
            if at != app.region.id.part {
                app.region.id.part = at;
            }
            let part = parts[at];
            // 部品の切り替え（◀ 部品 N / 個数 ▶）
            let row = rows.row(t::ROW_HEIGHT, 4.0);
            let prev = Rect::from_min_size(row.min, vec2(24.0, row.height()));
            let next = Rect::from_min_size(pos2(row.right() - 24.0, row.top()), vec2(24.0, row.height()));
            if w::icon_button(ui, prev, "id.part.prev", "expand_less", lang.pick("前の部品", "Previous part"), false, at > 0, 16.0).clicked() {
                app.apply(Action::Region(RegionAction::IdPart(at - 1)));
            }
            if w::icon_button(ui, next, "id.part.next", "expand_more", lang.pick("次の部品", "Next part"), false, at + 1 < parts.len(), 16.0).clicked() {
                app.apply(Action::Region(RegionAction::IdPart(at + 1)));
            }
            w::text(
                ui.painter(),
                Rect::from_min_max(pos2(row.left() + 28.0, row.top()), pos2(row.right() - 28.0, row.bottom())),
                &part_position(lang, at, parts.len()),
                t::LABEL,
                w::Align::Center,
            );
            // 色の見本と 16 進、自動に戻す
            let manual = crate::region::idcolor::manual_color(app, part);
            let shown = manual.or_else(|| app.id_part_hint(part));
            let row = rows.row(24.0, 4.0);
            let swatch = Rect::from_min_size(row.min + vec2(0.0, 1.0), vec2(36.0, row.height() - 2.0));
            let rgb = shown.unwrap_or(0x808080);
            let color = [((rgb >> 16) & 255) as f32 / 255.0, ((rgb >> 8) & 255) as f32 / 255.0, (rgb & 255) as f32 / 255.0, 1.0];
            w::color_swatch(
                ui,
                swatch,
                "id.part.swatch",
                color,
                lang.pick("この部品の ID の色", "This part's ID color"),
                false,
            );
            let hex_rect = Rect::from_min_size(pos2(swatch.right() + 6.0, row.top()), vec2(76.0, row.height()));
            let current = hex_of(rgb);
            let out = w::text_field(ui, hex_rect, "id.part.hex", &current, Some(lang.pick("16 進（#RRGGBB）で決める", "Set with hex (#RRGGBB)")), false);
            if let Some(text) = out.committed {
                if let Some(rgb) = parse_rgb(&text) {
                    app.apply(Action::Region(RegionAction::IdColor(IdColorOp::Set { part, rgb: Some(rgb) })));
                }
            }
            let reset = Rect::from_min_max(pos2(hex_rect.right() + 6.0, row.top()), row.max);
            if reset.width() > 30.0
                && w::button(ui, reset, "id.part.auto", lang.pick("自動", "Automatic"), false, free && manual.is_some(), None, None).clicked()
            {
                app.apply(Action::Region(RegionAction::IdColor(IdColorOp::Set { part, rgb: None })));
            }
        } else if found.is_none() {
            status_row(ui, rows, lang.pick("確かめています", "Checking"));
        } else if app.region_model().is_none() {
            status_row(ui, rows, &app.region_missing_reason());
        } else {
            status_row(ui, rows, lang.pick("このセットに部品がありません", "No parts in this set"));
        }
    }
    let row = rows.row(24.0, 4.0);
    if w::button(
        ui,
        row,
        "id.colors.reset",
        lang.pick("手動の色を全部やめる", "Reset all manual colors"),
        false,
        free && count > 0,
        None,
        None,
    )
    .clicked()
    {
        app.apply(Action::Region(RegionAction::IdColor(IdColorOp::ResetAll)));
    }
    if let Some([count_line, why]) = manual_state_lines(lang, count) {
        status_row(ui, rows, &count_line);
        let r = rows.row(t::ROW_HEIGHT, 2.0);
        let shown = w::fit(ui.painter(), &why, r.width(), t::LABEL_DIM);
        w::text(ui.painter(), r, &shown, t::LABEL_DIM.with_color(t::WARNING), w::Align::Left);
    }
}
