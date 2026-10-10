//! 範囲のツール（バケツ・ポリゴン塗りつぶし・ID の色で選択）の欄: オプションバー（塗る/消す・不透明度・許容。ID の色で選択は作成方法と許容）と、
//! 左のドックのツールプロパティ（バケツの許容・隣接・参照・色差・隙間閉じ・領域の拡縮・塗り残し、ID の色で選択の ID マップ）。範囲の種類
//! （近い色・三角形・メッシュの塊・UV アイランド・マテリアル）はサブツールの一覧（`subtool`）で選ぶ。値は `AppState::region` で、操作は
//! `Action::Region` を通す（キー・試験と同じ道）。画面には名前と値だけを出し、説明はツールチップ。

use egui::{pos2, vec2, Rect, Ui};

use super::color_window::{self, Pick};
use super::properties::{
    choice_buttons, group_label, slider_row, snap_symmetry_row, status_row, toggle_row,
    ChoiceButton,
};
use crate::engine::SelectionCombine;
use crate::region::idcolor::{hex_of, manual_state_line, parse_rgb};
use crate::region::{IdColorOp, RegionAction};
use crate::selection::{combine_name, combine_tooltip, SelAction, SelUiOp};
use crate::state::{Action, AppState, Tool};
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, NumberFormat, Rows, SliderSpec};

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
        &SliderSpec::new(
            lang.pick("不透明度", "Opacity"),
            0.0,
            100.0,
            NumberFormat::int("%"),
        ),
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
        (
            lang.pick("白（見せる）", "White (show)"),
            lang.pick("黒（隠す）", "Black (hide)"),
        )
    } else {
        (lang.pick("塗る", "Paint"), lang.pick("消す", "Erase"))
    };
    let (paint_tip, erase_tip) = if mask {
        (
            lang.pick(
                "マスクを白で塗る（レイヤーを見せる）",
                "Fill the mask with white (shows the layer)",
            ),
            lang.pick(
                "マスクを黒で塗る（レイヤーを隠す）",
                "Fill the mask with black (hides the layer)",
            ),
        )
    } else {
        (
            lang.pick(
                "描画色と不透明度で塗る",
                "Fill with the paint color and the opacity",
            ),
            lang.pick("透明にする", "Erase to transparent"),
        )
    };
    let width = |p: &egui::Painter, s: &str| w::text_width(p, s, t::LABEL) + 20.0;
    let pw = width(ui.painter(), paint);
    let ew = width(ui.painter(), erase);
    let erasing = app.region.erase;
    let a = cursor.next(pw);
    if w::button(
        ui,
        a,
        "region.paint",
        paint,
        !erasing,
        true,
        Some(paint_tip),
        None,
    )
    .clicked()
    {
        app.apply(Action::Region(RegionAction::Erase(false)));
    }
    cursor.x -= 6.0; // 塗る・消すは 1 組
    let b = cursor.next(ew);
    if w::button(
        ui,
        b,
        "region.erase",
        erase,
        erasing,
        true,
        Some(erase_tip),
        None,
    )
    .clicked()
    {
        app.apply(Action::Region(RegionAction::Erase(true)));
    }
}

fn tolerance_slider(
    ui: &mut Ui,
    app: &mut AppState,
    at: Rect,
    id: &str,
    max: f32,
    tip: &str,
    value: f32,
) -> Option<f32> {
    let lang = app.lang;
    let out = w::slider(
        ui,
        at,
        id,
        value,
        &SliderSpec::new(
            lang.pick("許容", "Tolerance"),
            0.0,
            max,
            NumberFormat::int(""),
        )
        .tooltip(tip),
    );
    out.changed.then(|| out.value.round().clamp(0.0, max))
}

/// 選択範囲の組み合わせ方（置き換え・足す・引く・重ねる）。選択のツールのオプションバーと同じ値（`AppState::sel`）を切り替える。
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
            Some(&combine_tooltip(lang, mode)),
            None,
        )
        .clicked()
        {
            app.apply(Action::Sel(SelAction::Ui(SelUiOp::Combine(mode))));
        }
    }
    cursor.x += 8.0;
}

/// オプションバーの中身（ツールのアイコンの右から）。バケツ・ポリゴン塗りつぶしは塗る/消すと不透明度（バケツの近い色は許容も）、ID の色で
/// 選択は作成方法と許容。範囲の種類はサブツール、ほかの値はツールプロパティ。
pub fn options(ui: &mut Ui, app: &mut AppState, r: Rect, x: f32) {
    let lang = app.lang;
    let mut cursor = Cursor {
        x,
        y: r.top() + 6.0,
        h: r.height() - 12.0,
    };
    match app.tool {
        Tool::Fill | Tool::PolygonFill => {
            paint_erase(ui, app, &mut cursor);
            let at = cursor.next(130.0);
            opacity_slider(ui, app, at);
            if app.tool == Tool::Fill && app.region.by_color {
                let at = cursor.next(150.0);
                let v = app.region.tolerance as f32;
                if let Some(v) = tolerance_slider(
                    ui,
                    app,
                    at,
                    "options.tolerance",
                    255.0,
                    lang.pick(
                        "押した画素の色との各成分の差の上限",
                        "The largest per-channel difference from the pressed pixel",
                    ),
                    v,
                ) {
                    app.apply(Action::Region(RegionAction::Tolerance(v as u8)));
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
                let shown = w::fit(
                    ui.painter(),
                    &reason,
                    (r.right() - cursor.x - 12.0).max(40.0),
                    t::LABEL_DIM,
                );
                let at = cursor.next(w::text_width(ui.painter(), &shown, t::LABEL_DIM) + 6.0);
                w::text(
                    ui.painter(),
                    at,
                    &shown,
                    t::LABEL_DIM.with_color(t::WARNING),
                    w::Align::Left,
                );
            }
        }
        _ => {}
    }
}

// ───────── ツールプロパティ ─────────

/// 塗る・消すの 1 行（マスクでは白・黒）。
fn paint_erase_row(ui: &mut Ui, app: &mut AppState, rows: &mut Rows) {
    let lang = app.lang;
    let mask = app.m2.edit_mask;
    let (paint, erase) = if mask {
        (
            lang.pick("白（見せる）", "White (show)"),
            lang.pick("黒（隠す）", "Black (hide)"),
        )
    } else {
        (lang.pick("塗る", "Paint"), lang.pick("消す", "Erase"))
    };
    let (paint_tip, erase_tip) = if mask {
        (
            lang.pick(
                "マスクを白で塗る（レイヤーを見せる）",
                "Fill the mask with white (shows the layer)",
            ),
            lang.pick(
                "マスクを黒で塗る（レイヤーを隠す）",
                "Fill the mask with black (hides the layer)",
            ),
        )
    } else {
        (
            lang.pick(
                "描画色と不透明度で塗る",
                "Fill with the paint color and the opacity",
            ),
            lang.pick("透明にする", "Erase to transparent"),
        )
    };
    let erasing = app.region.erase;
    let items = [
        ChoiceButton {
            id: "props.region.paint",
            label: paint,
            selected: !erasing,
            enabled: true,
            tooltip: Some(paint_tip),
        },
        ChoiceButton {
            id: "props.region.erase",
            label: erase,
            selected: erasing,
            enabled: true,
            tooltip: Some(erase_tip),
        },
    ];
    if let Some(i) = choice_buttons(ui, rows, &items) {
        app.apply(Action::Region(RegionAction::Erase(i == 1)));
    }
}

/// 不透明度の 1 行（ブラシと共通の値）。
fn opacity_row(ui: &mut Ui, app: &mut AppState, rows: &mut Rows) {
    let lang = app.lang;
    if let Some(v) = slider_row(
        ui,
        rows,
        "props.region.opacity",
        lang.pick("不透明度", "Opacity"),
        app.brush.opacity * 100.0,
        (0.0, 100.0),
        NumberFormat::int("%"),
        Some(lang.pick("ブラシと共通", "Shared with the brush")),
        true,
    ) {
        app.brush.opacity = v / 100.0;
    }
}

/// バケツ: 塗る・消す、不透明度、近い色なら許容・隣接・参照・色差・隙間閉じ・領域の拡縮・塗り残し。モデルの範囲なら領域の拡縮。
pub fn fill_props(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, _ctx: &egui::Context) {
    let lang = app.lang;
    paint_erase_row(ui, app, rows);
    opacity_row(ui, app, rows);
    if app.region.by_color {
        let tol = app.region.tolerance as f32;
        if let Some(v) = slider_row(
            ui,
            rows,
            "region.tolerance",
            lang.pick("許容", "Tolerance"),
            tol,
            (0.0, 255.0),
            NumberFormat::int(""),
            Some(lang.pick(
                "押した画素の色との各成分の差の上限",
                "The largest per-channel difference from the pressed pixel",
            )),
            true,
        ) {
            app.apply(Action::Region(RegionAction::Tolerance(v.round() as u8)));
        }
        let c = app.region.contiguous;
        if let Some(v) = toggle_row(
            ui,
            rows,
            "region.contiguous",
            lang.pick("隣接", "Contiguous"),
            c,
            None,
            true,
        ) {
            app.apply(Action::Region(RegionAction::Contiguous(v)));
        }
        bucket_properties(ui, app, rows);
    } else {
        if app.region_model().is_none() {
            status_row(ui, rows, &app.region_missing_reason());
        }
        if let Some(v) = slider_row(
            ui,
            rows,
            "bucket.margin",
            lang.pick("領域の拡縮", "Area scaling"),
            app.region.color.margin as f32,
            (-200.0, 200.0),
            NumberFormat::int(" px"),
            None,
            true,
        ) {
            app.apply(Action::Region(RegionAction::Margin(v.round() as i16)));
        }
    }
    if let Some(v) = snap_symmetry_row(
        ui,
        rows,
        lang,
        "region.snap-symmetry",
        app.region.snap_symmetry,
        if app.paints_only_in_3d() {
            Some(lang.pick("2D だけ", "2D only"))
        } else if app.region.by_color && app.region.color.leftovers {
            Some(lang.pick("塗り残しでは効きません", "Not used with leftover fill"))
        } else {
            None
        },
    ) {
        app.apply(Action::Region(RegionAction::SnapSymmetry(v)));
    }
    rows.space(4.0);
}

/// ポリゴン塗りつぶし: 塗る・消す、不透明度。
pub fn polygon_props(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, _ctx: &egui::Context) {
    paint_erase_row(ui, app, rows);
    opacity_row(ui, app, rows);
    if app.region_model().is_none() {
        status_row(ui, rows, &app.region_missing_reason());
    }
    rows.space(4.0);
}

/// ID の色で選択: 作成方法、許容、ID マップ（ベイク・部品の手動の色）。
pub fn id_props(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, _ctx: &egui::Context) {
    crate::selection::props::creation_row(ui, app, rows);
    id_section(ui, app, rows);
}

fn id_section(ui: &mut Ui, app: &mut AppState, rows: &mut Rows) {
    let lang = app.lang;
    // 使える状態は文で説明しない（ベイクのボタンの名前が「ベイクし直す」になる）。使えない理由だけを出す
    let usable = app.usable_id_map();
    if let Err(reason) = &usable {
        status_row(ui, rows, reason);
    }
    // ベイクのウィンドウを、ID マップにチェックを入れて開く
    let row = rows.row(24.0, 4.0);
    let label = if usable.is_ok() {
        lang.pick("ID マップをベイクし直す…", "Bake ID Map Again…")
    } else {
        lang.pick("ID マップをベイク…", "Bake ID Map…")
    };
    let baking = app.bake.is_baking();
    if w::button(
        ui,
        row,
        "id.bake",
        label,
        usable.is_err(),
        !baking && !app.is_stroking(),
        None,
        None,
    )
    .clicked()
    {
        app.apply(Action::Bake(crate::bake::BakeAction::Map(
            yolu_core::mesh_maps::MeshMapKind::Id,
            true,
        )));
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

/// 手動の ID の色（メッシュの塊ごと。文書の状態で、.ylp の正本の版 19 の塊に書く）。
fn manual_colors(ui: &mut Ui, app: &mut AppState, rows: &mut Rows) {
    let lang = app.lang;
    group_label(ui, rows, lang.pick("部品の手動の色", "Manual part colors"));
    let count = app.doc.id_colors().colors().len();
    let free = !app.is_stroking();
    if app.id_colors_foreign() {
        status_row(
            ui,
            rows,
            lang.pick(
                "別のモデルの手動の色です",
                "These manual colors belong to another model",
            ),
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
            let next = Rect::from_min_size(
                pos2(row.right() - 24.0, row.top()),
                vec2(24.0, row.height()),
            );
            if w::icon_button(
                ui,
                prev,
                "id.part.prev",
                "expand_less",
                lang.pick("前の部品", "Previous part"),
                false,
                at > 0,
                16.0,
            )
            .clicked()
            {
                app.apply(Action::Region(RegionAction::IdPart(at - 1)));
            }
            if w::icon_button(
                ui,
                next,
                "id.part.next",
                "expand_more",
                lang.pick("次の部品", "Next part"),
                false,
                at + 1 < parts.len(),
                16.0,
            )
            .clicked()
            {
                app.apply(Action::Region(RegionAction::IdPart(at + 1)));
            }
            w::text(
                ui.painter(),
                Rect::from_min_max(
                    pos2(row.left() + 28.0, row.top()),
                    pos2(row.right() - 28.0, row.bottom()),
                ),
                &part_position(lang, at, parts.len()),
                t::LABEL,
                w::Align::Center,
            );
            // 色の見本と 16 進、自動に戻す
            let manual = crate::region::idcolor::manual_color(app, part);
            let shown = manual.or_else(|| app.id_part_hint(part));
            let row = rows.row(24.0, 4.0);
            let swatch =
                Rect::from_min_size(row.min + vec2(0.0, 1.0), vec2(36.0, row.height() - 2.0));
            let rgb = shown.unwrap_or(0x808080);
            let editable = app.can_edit();
            part_color(ui, app, swatch, part, manual, rgb, editable);
            let hex_rect = Rect::from_min_size(
                pos2(swatch.right() + 6.0, row.top()),
                vec2(76.0, row.height()),
            );
            let current = hex_of(rgb);
            let out = w::text_field(
                ui,
                hex_rect,
                "id.part.hex",
                &current,
                Some(lang.pick("16 進（#RRGGBB）で決める", "Set with hex (#RRGGBB)")),
                false,
            );
            if let Some(text) = out.committed {
                if let Some(rgb) = parse_rgb(&text) {
                    app.apply(Action::Region(RegionAction::IdColor(IdColorOp::Set {
                        part,
                        rgb: Some(rgb),
                    })));
                }
            }
            let reset = Rect::from_min_max(pos2(hex_rect.right() + 6.0, row.top()), row.max);
            if reset.width() > 30.0
                && w::button(
                    ui,
                    reset,
                    "id.part.auto",
                    lang.pick("自動", "Automatic"),
                    false,
                    free && manual.is_some(),
                    None,
                    None,
                )
                .clicked()
            {
                app.apply(Action::Region(RegionAction::IdColor(IdColorOp::Set {
                    part,
                    rgb: None,
                })));
            }
        } else if found.is_none() {
            status_row(ui, rows, lang.pick("確かめています", "Checking"));
        } else if app.region_model().is_none() {
            status_row(ui, rows, &app.region_missing_reason());
        }
        // 部品が無いセットは、空のまま（文字を置かない）
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
    if let Some(count_line) = manual_state_line(lang, count) {
        status_row(ui, rows, &count_line);
    }
}

/// 部品の ID の色の見本。押すと色のウィンドウ（相手は文書と部品ごと）。ウィンドウの変更はその場で当て、ドラッグ 1 回を 1 回の取り消しにまとめる。
/// ウィンドウがこの文書の部品の色を相手にしている間に部品を替えたら、ウィンドウはそのままで相手を新しい部品へ替える。自動の色を見せているときに
/// 開いて Esc で戻したら、自動へ戻す。
fn part_color(
    ui: &mut Ui,
    app: &mut AppState,
    swatch: Rect,
    part: usize,
    manual: Option<u32>,
    rgb: u32,
    enabled: bool,
) {
    let lang = app.lang;
    let ctx = ui.ctx().clone();
    let doc = app.doc.id();
    let target_of = |part: usize| egui::Id::new(("id.part.color", doc, part));
    let target = target_of(part);
    let auto_id = target.with("auto");
    let name = lang.pick("部品の ID の色", "Part ID Color");
    let current = Pick::rgb([(rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8]);
    let last_id = egui::Id::new(("id.part.color.last", doc));
    let last = ctx.data(|d| d.get_temp::<usize>(last_id));
    ctx.data_mut(|d| d.insert_temp(last_id, part));
    let was_target = color_window::is_target(&ctx, target);
    if enabled && !was_target && last.is_some_and(|l| color_window::is_target(&ctx, target_of(l))) {
        color_window::open(&ctx, target, name, swatch, ui.clip_rect(), current);
    }
    let update = color_window::field(
        ui,
        swatch,
        target,
        name,
        current,
        lang.pick("この部品の ID の色", "This part's ID color"),
        enabled,
    );
    if !was_target && color_window::is_target(&ctx, target) {
        ctx.data_mut(|d| d.insert_temp(auto_id, manual.is_none()));
    }
    let Some(u) = update else {
        return;
    };
    let auto = ctx.data(|d| d.get_temp::<bool>(auto_id)).unwrap_or(false);
    let [r, g, b] = u.pick.rgb;
    let next = (u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b);
    let op = if u.reverted && auto {
        manual.map(|_| IdColorOp::Set { part, rgb: None })
    } else if manual == Some(next) {
        None
    } else if u.dragging {
        Some(IdColorOp::Drag { part, rgb: next })
    } else {
        Some(IdColorOp::Set {
            part,
            rgb: Some(next),
        })
    };
    if let Some(op) = op {
        app.apply(Action::Region(RegionAction::IdColor(op)));
    }
    if u.done {
        app.apply(Action::Region(RegionAction::IdColor(IdColorOp::EndDrag)));
    }
}

fn bucket_properties(ui: &mut Ui, app: &mut AppState, rows: &mut Rows) {
    use crate::region::color::{Distance, Reference};
    let lang = app.lang;
    for (value, ja, en) in [
        (Reference::Editing, "編集しているレイヤー", "Editing layer"),
        (Reference::Visible, "全部のレイヤー", "All visible layers"),
        (Reference::Marked, "参照レイヤー", "Reference layers"),
    ] {
        let current = if app.region.sample_all {
            Reference::Visible
        } else {
            app.region.color.reference
        };
        if toggle_row(
            ui,
            rows,
            ja,
            lang.pick(ja, en),
            current == value,
            None,
            true,
        ) == Some(true)
        {
            app.apply(Action::Region(RegionAction::Reference(value)));
        }
    }
    let p = app.region.color.distance == Distance::Perceptual;
    if let Some(v) = toggle_row(
        ui,
        rows,
        "bucket.distance",
        lang.pick("知覚の色差", "Perceptual difference"),
        p,
        Some(lang.pick(
            "オフ: RGB の差の最大。オン: 人の見え方に合わせた色差の近似",
            "Off: largest RGB difference. On: approximate perceptual color difference",
        )),
        true,
    ) {
        app.apply(Action::Region(RegionAction::Distance(if v {
            Distance::Perceptual
        } else {
            Distance::Rgb
        })));
    }
    for (id, ja, en, value, min, max) in [
        (
            "bucket.gap",
            "隙間閉じ",
            "Close gap",
            app.region.color.gap as f32,
            0.0,
            32.0,
        ),
        (
            "bucket.margin",
            "領域の拡縮",
            "Area scaling",
            app.region.color.margin as f32,
            -200.0,
            200.0,
        ),
    ] {
        if let Some(v) = slider_row(
            ui,
            rows,
            id,
            lang.pick(ja, en),
            value,
            (min, max),
            NumberFormat::int(" px"),
            None,
            true,
        ) {
            let action = if id == "bucket.gap" {
                RegionAction::Gap(v.round() as u8)
            } else {
                RegionAction::Margin(v.round() as i16)
            };
            app.apply(Action::Region(action));
        }
    }
    if let Some(v) = toggle_row(
        ui,
        rows,
        "bucket.leftovers",
        lang.pick("塗り残し部分に塗る", "Paint unfilled areas"),
        app.region.color.leftovers,
        Some(lang.pick(
            "なぞった範囲に触れる小さな閉領域の透明・白い部分",
            "Transparent or white pixels of small enclosed regions touched by the stroke",
        )),
        true,
    ) {
        app.apply(Action::Region(RegionAction::Leftovers(v)));
    }
    if app.region.color.leftovers {
        if let Some(v) = slider_row(
            ui,
            rows,
            "bucket.area",
            lang.pick("囲みの最大面積", "Maximum enclosed area"),
            app.region.color.max_area as f32,
            (1.0, 65536.0),
            NumberFormat::int(" px²"),
            None,
            true,
        ) {
            app.apply(Action::Region(RegionAction::MaxArea(v.round() as u32)));
        }
    }
}
