//! ドラッグ＆ドロップの入れ先の下書き（A・B・C）が共通で使う物: ウィンドウ全体のダミー、ダミーのテクスチャセットとレイヤーセット、
//! 選びのドロップダウン（メニュー付き）、ウィンドウの枠と下の帯。部品は `yolu_app::ui` の物だけ。

use egui::{pos2, vec2, Color32, Id, Order, Rect, Ui, Vec2};
use yolu_app::ui::menu;
use yolu_app::ui::theme as t;
use yolu_app::ui::widgets::{self as w, Align};
use yolu_app::ui::window::{self, Frame, Spec};

use super::layers_common as parts;
use crate::page::SketchCtx;

/// 日本語・英語の対。
pub type Pair = (&'static str, &'static str);

pub const MATERIAL: Pair = ("マテリアル", "Material");
pub const SHADOW: Pair = ("影のマスク", "Shadow Mask");
pub const NEW_LAYER_SET: Pair = ("＋新しいレイヤーセット", "+ New Layer Set");

/// ダミーのテクスチャセット 1 つと、その中のレイヤーセット。
pub struct TexSet {
    pub name: Pair,
    pub sets: &'static [Pair],
}

pub const TEX_SETS: [TexSet; 3] = [
    TexSet {
        name: ("肌", "Skin"),
        sets: &[MATERIAL, SHADOW],
    },
    TexSet {
        name: ("服", "Clothes"),
        sets: &[MATERIAL],
    },
    TexSet {
        name: ("髪", "Hair"),
        sets: &[MATERIAL, SHADOW],
    },
];

pub fn name(ctx: &SketchCtx, pair: Pair) -> &'static str {
    ctx.pick(pair.0, pair.1)
}

/// 取り込む側の取り込むボタンの名前（今の PSD の取り込みのウィンドウと同じ）。
pub const CANCEL: Pair = ("やめる", "Cancel");
pub const IMPORT: Pair = ("取り込む", "Import");

// ───────── 背景のダミー ─────────

/// ウィンドウ全体のダミー（上にメニューバーとオプションの帯、左にツール、真ん中にキャンバスと 3D ビュー、右にテクスチャセットとレイヤーのパネル、下に状態の帯）。
/// タブの本体ではなく、画面全体の背景のレイヤーに描く。
pub fn backdrop(ui: &Ui, ctx: &mut SketchCtx) {
    let screen = ui.ctx().content_rect();
    egui::Area::new(Id::new("dropin.backdrop"))
        .order(Order::Background)
        .fixed_pos(screen.min)
        .constrain(false)
        .show(ui.ctx(), |ui| {
            ui.set_clip_rect(screen);
            draw_backdrop(ui, ctx, screen);
        });
}

fn draw_backdrop(ui: &mut Ui, ctx: &mut SketchCtx, screen: Rect) {
    let p = ui.painter().clone();
    w::fill(&p, screen, t::WINDOW_BG);

    // 上: メニューバーとオプションの帯
    let bar = Rect::from_min_size(screen.min, vec2(screen.width(), t::MENU_BAR_HEIGHT));
    let titles = [
        ctx.pick("ファイル", "File"),
        ctx.pick("編集", "Edit"),
        ctx.pick("レイヤー", "Layer"),
        ctx.pick("表示", "View"),
    ];
    let _ = menu::menu_bar(ui, bar, &titles, None);
    let options = Rect::from_min_size(
        pos2(screen.left(), bar.bottom()),
        vec2(screen.width(), t::OPTIONS_BAR_HEIGHT),
    );
    w::fill(&p, options, t::PANEL_BG);
    w::hline(
        &p,
        options.left(),
        options.right(),
        options.bottom() - 1.0,
        t::BORDER,
    );
    for (x, width) in [(12.0, 110.0), (132.0, 80.0), (222.0, 140.0)] {
        let r = Rect::from_min_size(
            pos2(options.left() + x, options.top() + 7.0),
            vec2(width, options.height() - 14.0),
        );
        w::rounded(&p, r, t::CONTROL_BG, 3.0);
    }

    // 下: 状態の帯
    let status = Rect::from_min_max(
        pos2(screen.left(), screen.bottom() - t::STATUS_BAR_HEIGHT),
        screen.max,
    );
    w::fill(&p, status, t::PANEL_HEADER);
    w::hline(&p, status.left(), status.right(), status.top(), t::BORDER);

    let body = Rect::from_min_max(
        pos2(screen.left(), options.bottom()),
        status.left_top() + vec2(screen.width(), 0.0),
    );

    // 左: ツール
    let strip = Rect::from_min_size(body.min, vec2(t::TOOL_STRIP_WIDTH, body.height()));
    w::fill(&p, strip, t::PANEL_BG);
    w::vline(
        &p,
        strip.right() - 1.0,
        strip.top(),
        strip.bottom(),
        t::BORDER,
    );
    let tools: [(&str, &str, &str); 8] = [
        ("brush", "ブラシ", "Brush"),
        ("eraser", "消しゴム", "Eraser"),
        ("fill", "塗りつぶし", "Fill"),
        ("gradient", "グラデーション", "Gradient"),
        ("eyedropper", "スポイト", "Eyedropper"),
        ("lasso", "投げ縄", "Lasso"),
        ("move", "移動", "Move"),
        ("shape", "図形", "Shape"),
    ];
    for (i, (id, ja, en)) in tools.iter().enumerate() {
        let r = Rect::from_min_size(
            pos2(strip.left() + 4.0, strip.top() + 6.0 + i as f32 * 38.0),
            vec2(36.0, 34.0),
        );
        let _ = w::tool_button(ui, r, id, ctx.pick(ja, en), i == 0);
    }

    // 右: パネル
    let dock = Rect::from_min_max(pos2(body.right() - t::DOCK_WIDTH, body.top()), body.max);
    w::fill(&p, dock, t::PANEL_BG);
    w::vline(&p, dock.left(), dock.top(), dock.bottom(), t::BORDER);
    let tex_panel = Rect::from_min_size(dock.min, vec2(dock.width(), 22.0 + 5.0 * 26.0 + 6.0));
    texture_sets_panel(ui, ctx, tex_panel);
    let layer_panel = Rect::from_min_max(pos2(dock.left(), tex_panel.bottom()), dock.max);
    layers_panel(ui, ctx, layer_panel);

    // 真ん中: キャンバスと 3D ビュー
    let center = Rect::from_min_max(
        pos2(strip.right(), body.top()),
        pos2(dock.left(), body.bottom()),
    );
    let half = ((center.width() - 2.0) / 2.0).floor();
    let canvas = Rect::from_min_size(center.min, vec2(half, center.height()));
    let view3d = Rect::from_min_max(pos2(canvas.right() + 2.0, center.top()), center.max);
    w::fill(&p, canvas, t::CANVAS_BG);
    let side = (canvas.width().min(canvas.height()) * 0.7).floor();
    let sheet = Rect::from_center_size(canvas.center(), vec2(side, side));
    w::checker(&p, sheet, 12.0);
    w::outline(&p, sheet, t::BORDER, 1.0, 0.0);
    w::fill(&p, view3d, t::CANVAS_BG);
    // 3D のモデルの所は、灰色の面と輪郭の線だけ
    let radius = (view3d.width().min(view3d.height()) * 0.28).floor();
    p.circle_filled(view3d.center(), radius, Color32::from_gray(74));
    p.circle_stroke(
        view3d.center(),
        radius,
        egui::Stroke::new(1.5, Color32::from_gray(110)),
    );
    p.circle_stroke(
        view3d.center(),
        radius * 0.55,
        egui::Stroke::new(1.0, Color32::from_gray(92)),
    );
}

fn texture_sets_panel(ui: &mut Ui, ctx: &SketchCtx, r: Rect) {
    let p = ui.painter().clone();
    w::panel_title(
        &p,
        Rect::from_min_size(r.min, vec2(r.width(), 22.0)),
        ctx.pick("テクスチャセット", "Texture Sets"),
        Some("texture"),
    );
    let mut y = r.top() + 22.0;
    let mut row = |depth: usize, label: &str, selected: bool, set: bool| {
        let rect = Rect::from_min_size(pos2(r.left(), y), vec2(r.width(), 26.0));
        y += 26.0;
        if selected {
            w::fill(&p, rect, t::ACCENT_SOFT);
            w::fill(
                &p,
                Rect::from_min_size(rect.min, vec2(3.0, rect.height())),
                t::ACCENT,
            );
        }
        w::hline(
            &p,
            rect.left(),
            rect.right(),
            rect.bottom() - 1.0,
            t::BORDER,
        );
        let x = rect.left() + 10.0 + 18.0 * depth as f32;
        let mark = Rect::from_min_size(pos2(x, rect.top() + 4.0), vec2(18.0, 18.0));
        if set {
            w::icon(&p, mark, "layers", t::TEXT_DIM, 14.0);
        } else {
            w::rounded(&p, mark, Color32::from_gray(86), 3.0);
            w::outline(&p, mark, t::BORDER, 1.0, 3.0);
        }
        w::text(
            &p,
            Rect::from_min_max(pos2(mark.right() + 6.0, rect.top()), rect.max),
            label,
            t::LABEL.with_color(if selected { Color32::WHITE } else { t::TEXT }),
            Align::Left,
        );
    };
    let skin = &TEX_SETS[0];
    row(0, ctx.pick(skin.name.0, skin.name.1), false, false);
    row(1, ctx.pick(MATERIAL.0, MATERIAL.1), true, true);
    row(1, ctx.pick(SHADOW.0, SHADOW.1), false, true);
    for set in &TEX_SETS[1..] {
        row(0, ctx.pick(set.name.0, set.name.1), false, false);
    }
}

fn layers_panel(ui: &mut Ui, ctx: &mut SketchCtx, r: Rect) {
    let p = ui.painter().clone();
    w::panel_title(
        &p,
        Rect::from_min_size(r.min, vec2(r.width(), 22.0)),
        ctx.pick("レイヤー", "Layers"),
        Some("layers"),
    );
    let list = Rect::from_min_max(pos2(r.left(), r.top() + 22.0), r.max);
    w::fill(&p, list, t::CONTROL_BG);
    for (i, d) in parts::ROWS.iter().enumerate() {
        let row = Rect::from_min_size(
            pos2(list.left(), list.top() + i as f32 * parts::ROW_HEIGHT),
            vec2(list.width(), parts::ROW_HEIGHT),
        );
        if row.bottom() > list.bottom() {
            break;
        }
        parts::row(ui, ctx, list, row, i, d, 26.0);
    }
}

// ───────── ウィンドウの枠 ─────────

/// 画面の真ん中に出るウィンドウ（モーダル）。撮る間は動かさず、ウィンドウでは見出しのドラッグで動く。
pub fn show_window(
    ui: &Ui,
    ctx: &mut SketchCtx,
    name: &'static str,
    title: &str,
    icon: &str,
    size: Vec2,
    contents: impl FnOnce(&mut Ui, &Frame, &mut SketchCtx),
) {
    let id = Id::new(("dropin.window", name));
    let offset_id = id.with("offset");
    let mut offset: Vec2 = if ctx.still {
        Vec2::ZERO
    } else {
        ui.ctx()
            .data(|d| d.get_temp(offset_id))
            .unwrap_or(Vec2::ZERO)
    };
    let spec = Spec {
        title,
        icon: Some(icon),
        size,
        modal: true,
        close_label: ctx.pick("ウィンドウを閉じる", "Close Window"),
    };
    let _ = window::show(ui.ctx(), id, &spec, &mut offset, false, |ui, frame| {
        contents(ui, frame, ctx)
    });
    if !ctx.still {
        ui.ctx().data_mut(|d| d.insert_temp(offset_id, offset));
    }
}

pub const FOOTER: f32 = 48.0;

/// 下の帯（今の取り込みのウィンドウと同じ。右からボタンを並べる。各ボタンは名前・青いか・押せるか）。押されたボタンの番号（左から）を返す。
pub fn footer(ui: &mut Ui, id: Id, body: Rect, buttons: &[(&str, bool, bool)]) -> Option<usize> {
    let p = ui.painter().clone();
    let footer = Rect::from_min_max(pos2(body.left(), body.bottom() - FOOTER), body.max);
    w::fill(&p, footer, t::PANEL_HEADER);
    w::hline(&p, footer.left(), footer.right(), footer.top(), t::BORDER);
    let mut pressed = None;
    let mut x = footer.right() - 14.0;
    for (i, (label, primary, enabled)) in buttons.iter().enumerate().rev() {
        let width = w::text_width(&p, label, t::LABEL) + 32.0;
        let r = Rect::from_min_size(pos2(x - width, footer.top() + 10.0), vec2(width, 28.0));
        x = r.left() - 8.0;
        if w::button(
            ui,
            r,
            id.with(("button", i)),
            label,
            *primary,
            *enabled,
            None,
            None,
        )
        .clicked()
        {
            pressed = Some(i);
        }
    }
    pressed
}

// ───────── 入れ先の選び ─────────

/// 入れ先 1 つ。何も選んでいない形（既定）は `Option` の `None` で持つ。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Dest {
    /// レイヤーにはせず、アセットの欄に入れるだけ。
    Asset,
    /// テクスチャセットの行を選んだ形（そのテクスチャセットの新しいレイヤーセットとして入る）。
    NewSet(usize),
    /// 既にあるレイヤーセット（テクスチャセットの番号・レイヤーセットの番号。その一番上に入る）。
    Set(usize, usize),
}

const TAG_NONE: usize = usize::MAX;
const TAG_ASSET: usize = usize::MAX - 1;

/// 入れ先を値に覚える（`key` ごとに 2 つの番号。`None` は選んでいない）。
pub fn dest_set(ctx: &mut SketchCtx, key: &str, dest: Option<Dest>) {
    let (tex, set) = match dest {
        None => (TAG_NONE, TAG_NONE),
        Some(Dest::Asset) => (TAG_ASSET, TAG_NONE),
        Some(Dest::NewSet(tex)) => (tex, TAG_NONE),
        Some(Dest::Set(tex, set)) => (tex, set),
    };
    ctx.values.set_index(&format!("{key}.tex"), tex);
    ctx.values.set_index(&format!("{key}.set"), set);
}

pub fn dest_get(ctx: &SketchCtx, key: &str) -> Option<Dest> {
    let tex = ctx.values.index(&format!("{key}.tex"), TAG_NONE);
    let set = ctx.values.index(&format!("{key}.set"), TAG_NONE);
    match tex {
        TAG_NONE => None,
        TAG_ASSET => Some(Dest::Asset),
        t if t < TEX_SETS.len() && set < TEX_SETS[t].sets.len() => Some(Dest::Set(t, set)),
        t if t < TEX_SETS.len() => Some(Dest::NewSet(t)),
        _ => None,
    }
}

pub const ASSET_SHORT: Pair = ("アセット", "Assets");
pub const ASSET_ONLY: Pair = ("アセットに入れるだけ", "Assets Only");
/// アセットに入れられないファイルの行（PSD）に出す短い理由。
pub const ASSET_REASON: Pair = ("PSD は不可", "No PSD");

/// 閉じた形の 1 行の表示: 「肌 › 影のマスク」「肌 › ＋新しいレイヤーセット」「アセット」。
pub fn dest_text(ctx: &SketchCtx, dest: Dest) -> String {
    match dest {
        Dest::Asset => name(ctx, ASSET_SHORT).to_owned(),
        Dest::NewSet(tex) => format!(
            "{} › {}",
            name(ctx, TEX_SETS[tex].name),
            name(ctx, NEW_LAYER_SET)
        ),
        Dest::Set(tex, set) => format!(
            "{} › {}",
            name(ctx, TEX_SETS[tex].name),
            name(ctx, TEX_SETS[tex].sets[set])
        ),
    }
}

const POPUP_ROW: f32 = 28.0;
const POPUP_PAD: f32 = 6.0;

/// 入れ先のドロップダウン（押すと、「アセットに入れるだけ」の行と、テクスチャセットごとの見出しの下にレイヤーセットを並べた 1 つの一覧が開く）。
/// 一覧のテクスチャセットの行は選べて、選ぶと新しいレイヤーセットとして入る（右に「+」の印）。レイヤーセットの行は、その一番上に入る。
/// 選んでいない形は箱が空。`asset_ok` が false の行（PSD）は、アセットの行を押せなくして短い理由を出す。
/// 選ばれたときだけ入れ先を返し、値（`key`）にも覚える。撮る間は `open_in_still` のとき、最初のフレームで一覧を開いた形にする。
#[allow(clippy::too_many_arguments)]
pub fn dest_choice(
    ui: &mut Ui,
    ctx: &mut SketchCtx,
    r: Rect,
    key: &str,
    label: Option<&str>,
    label_width: f32,
    enabled: bool,
    asset_ok: bool,
    open_in_still: bool,
) -> Option<Dest> {
    let current = dest_get(ctx, key);
    let shown = match current {
        // 押せない行は空（読めないファイルの行）
        _ if !enabled => String::new(),
        Some(d) => dest_text(ctx, d),
        None => String::new(),
    };
    let (response, anchor) = w::dropdown(
        ui,
        r,
        ("dropin.dest", key),
        label,
        &shown,
        None,
        enabled,
        label_width,
    );
    let open_id = Id::new(("dropin.dest.open", key));
    let seen_id = Id::new(("dropin.dest.seen", key));
    let mut open = ui.data(|d| d.get_temp::<bool>(open_id)).unwrap_or(false);
    let first = !ui.data(|d| d.get_temp::<bool>(seen_id)).unwrap_or(false);
    if response.clicked() || (ctx.still && open_in_still && first) {
        open = true;
        ui.data_mut(|d| d.insert_temp(seen_id, true));
    }
    let mut chosen = None;
    if open {
        let id = Id::new(("dropin.dest.popup", key));
        match dest_popup(ui.ctx(), ctx, id, anchor, current, asset_ok) {
            Popup::Open => {}
            Popup::Chosen(d) => {
                dest_set(ctx, key, Some(d));
                chosen = Some(d);
                open = false;
            }
            Popup::Close => open = false,
        }
        // 撮る間は、開いた形のまま残す
        if ctx.still && open_in_still {
            open = true;
        }
    }
    ui.data_mut(|d| d.insert_temp(open_id, open));
    chosen
}

enum Popup {
    Open,
    Chosen(Dest),
    Close,
}

/// 一覧の行（入れ先と、字下げの段）。頭にアセットの行、その下にテクスチャセットの行と、そのレイヤーセットの行。
fn popup_rows() -> Vec<(Dest, usize)> {
    let mut rows = vec![(Dest::Asset, 0)];
    for (tex, set) in TEX_SETS.iter().enumerate() {
        rows.push((Dest::NewSet(tex), 0));
        for i in 0..set.sets.len() {
            rows.push((Dest::Set(tex, i), 1));
        }
    }
    rows
}

fn dest_popup(
    egui_ctx: &egui::Context,
    ctx: &SketchCtx,
    id: Id,
    anchor: Rect,
    current: Option<Dest>,
    asset_ok: bool,
) -> Popup {
    let screen = egui_ctx.content_rect();
    let rows = popup_rows();
    let size = vec2(
        anchor.width().max(280.0),
        rows.len() as f32 * POPUP_ROW + 2.0 * POPUP_PAD,
    );
    // 箱の下に出す。入らなければ上
    let below = anchor.bottom() + 2.0;
    let y = if below + size.y <= screen.bottom() - 8.0 {
        below
    } else {
        (anchor.top() - 2.0 - size.y).max(screen.top() + 8.0)
    };
    let x = anchor.left().min(screen.right() - 8.0 - size.x).max(8.0);
    let rect = Rect::from_min_size(pos2(x, y), size);
    let mut result = Popup::Open;
    // 外を押したら閉じるだけで、下の部品へは渡さない
    egui::Area::new(id.with("blocker"))
        .order(Order::Foreground)
        .fixed_pos(screen.min)
        .constrain(false)
        .interactable(true)
        .show(egui_ctx, |ui| {
            if ui
                .interact(screen, id.with("hit"), egui::Sense::click_and_drag())
                .clicked()
            {
                result = Popup::Close;
            }
        });
    if egui_ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        result = Popup::Close;
    }
    egui::Area::new(id)
        .order(Order::Tooltip)
        .fixed_pos(rect.min)
        .constrain(false)
        .show(egui_ctx, |ui| {
            let p = ui.painter().clone();
            for i in (1..=5).rev() {
                let f = i as f32;
                p.rect_filled(
                    rect.expand(f).translate(vec2(0.0, 2.0)),
                    6.0 + f,
                    Color32::from_black_alpha(20),
                );
            }
            w::rounded(&p, rect, t::MENU_BG, 6.0);
            w::outline(&p, rect, t::SEPARATOR, 1.0, 6.0);
            for (i, (dest, depth)) in rows.iter().enumerate() {
                let row = Rect::from_min_size(
                    pos2(
                        rect.left() + 4.0,
                        rect.top() + POPUP_PAD + i as f32 * POPUP_ROW,
                    ),
                    vec2(rect.width() - 8.0, POPUP_ROW),
                );
                if *depth == 0 && i > 0 {
                    w::hline(
                        &p,
                        rect.left() + 8.0,
                        rect.right() - 8.0,
                        row.top() - 0.5,
                        t::BORDER,
                    );
                }
                let usable = *dest != Dest::Asset || asset_ok;
                let response = ui.interact(
                    row,
                    id.with(("row", i)),
                    if usable {
                        egui::Sense::click()
                    } else {
                        egui::Sense::hover()
                    },
                );
                let is_current = current == Some(*dest);
                if is_current {
                    w::rounded(&p, row, t::ACCENT_SOFT, 4.0);
                    w::fill(
                        &p,
                        Rect::from_min_size(
                            row.min + vec2(0.0, 4.0),
                            vec2(3.0, row.height() - 8.0),
                        ),
                        t::ACCENT,
                    );
                } else if usable && response.hovered() {
                    w::rounded(&p, row, t::CONTROL_HOVER, 4.0);
                }
                match *dest {
                    // アセットの行: アイコンと名前。入れられないときは薄くして、右に短い理由
                    Dest::Asset => {
                        let color = if usable { t::TEXT } else { t::TEXT_DISABLED };
                        w::icon(
                            &p,
                            Rect::from_min_size(
                                pos2(row.left() + 12.0, row.top()),
                                vec2(18.0, row.height()),
                            ),
                            "library",
                            color,
                            15.0,
                        );
                        w::text(
                            &p,
                            Rect::from_min_max(pos2(row.left() + 36.0, row.top()), row.max),
                            name(ctx, ASSET_ONLY),
                            t::LABEL.with_color(color),
                            Align::Left,
                        );
                        if !usable {
                            w::text(
                                &p,
                                Rect::from_min_max(row.min, pos2(row.right() - 12.0, row.bottom())),
                                name(ctx, ASSET_REASON),
                                t::LABEL_DIM.with_color(t::TEXT_DISABLED),
                                Align::Right,
                            );
                        }
                    }
                    // テクスチャセットの行: 名前（太字）と、右に新しいレイヤーセットの印
                    Dest::NewSet(tex) => {
                        w::text(
                            &p,
                            Rect::from_min_max(pos2(row.left() + 12.0, row.top()), row.max),
                            name(ctx, TEX_SETS[tex].name),
                            t::LABEL_BOLD,
                            Align::Left,
                        );
                        w::icon(
                            &p,
                            Rect::from_min_size(
                                pos2(row.right() - 28.0, row.top()),
                                vec2(20.0, row.height()),
                            ),
                            "add",
                            t::TEXT_DIM,
                            16.0,
                        );
                    }
                    Dest::Set(tex, set) => {
                        let x = row.left() + 12.0 + 18.0 * *depth as f32;
                        w::icon(
                            &p,
                            Rect::from_min_size(pos2(x, row.top()), vec2(18.0, row.height())),
                            "layers",
                            t::TEXT_DIM,
                            15.0,
                        );
                        w::text(
                            &p,
                            Rect::from_min_max(pos2(x + 24.0, row.top()), row.max),
                            name(ctx, TEX_SETS[tex].sets[set]),
                            t::LABEL,
                            Align::Left,
                        );
                    }
                }
                if usable && response.clicked() {
                    result = Popup::Chosen(*dest);
                }
            }
        });
    result
}

/// 灰色の画像のサムネイル（素材が無いので灰色の四角）。
pub fn image_thumb(p: &egui::Painter, r: Rect) {
    w::checker(p, r, 4.0);
    w::rounded(p, r.shrink(2.0), Color32::from_gray(92), 2.0);
    w::outline(p, r, t::BORDER, 1.0, 0.0);
}
