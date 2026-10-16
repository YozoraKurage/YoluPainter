//! 新規プロジェクト・プロジェクトの構成のウィンドウ（Unity 版の `NewProjectWindow` と同じ 1 つのウィンドウ）と、テクスチャセットを消す確認のウィンドウを描く。
//! 値は `AppState::np.window` そのもので、部品は `NpAction` を返し、描いたあとにまとめて当てる（描く途中で状態を借りない）。
//!
//! 文言は名前・状態・短い理由だけ。使い方の説明・空の欄の案内は置かず、説明はツールチップ。モデルの 3D の見本はまだ無い
//! （マテリアルの一覧に、それを使うメッシュの名前を添える）。

use std::path::Path;

use egui::{pos2, vec2, Id, Key, Rect, Sense, Ui, UiBuilder, Vec2};

use super::configure::{resampling_name, unused_groups, ConfirmKind, Plan};
use super::{
    size_text, DraftOp, Dropdown, Group, NpAction, NpWindow, Prep, SetDraft, Template, MAX_SETS,
    RESOLUTIONS,
};
use crate::engine::{CanvasResampling, NormalYDirection};
use crate::lang::Lang;
use crate::state::AppState;
use crate::ui::menu::{self, Entry, PopupOutcome, PopupState};
use crate::ui::scroll::Scroll;
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, Align};
use crate::ui::window::{self, Frame, Spec};

const WIDTH: f32 = 560.0;
const PAD: f32 = 16.0;
const ROW: f32 = 24.0;
const GAP: f32 = 8.0;
const LABEL_W: f32 = 168.0;
const FOOT: f32 = 52.0;
const SET_ROW: f32 = 26.0;
const LIST_ROWS: usize = 6;
const CARD_ROWS: usize = 8;

/// 開いているドロップダウン（`UiState::np_dropdown`。描く間だけ取り出す）。
type OpenDropdown = Option<(Dropdown, PopupState)>;

fn main_id() -> Id {
    Id::new("yolu.newproject")
}

/// 最後に描いたウィンドウの矩形（画面の点。開いていなければ None）。試験が位置を知るために読む。
pub fn last_rect(ctx: &egui::Context) -> Option<Rect> {
    window::last_rect(ctx, main_id())
}

/// ウィンドウが見せるもの（描く前に集める。描く途中で状態を借りない）。
struct View {
    lang: Lang,
    groups: Vec<Group>,
    /// 選んでいる・今のモデルの名前（無ければ None。欄は空のまま）。
    model_name: Option<String>,
    model_tip: String,
    /// 構成: 今のモデルのファイルを読み直せる。
    can_reload: bool,
    /// 構成: Live Link のモデル（Unity のシーンのもの。ここでは替えない）。
    link: bool,
    /// セットの無いマテリアルの組の数（構成）。
    unused: usize,
    /// モデルに無いセットの数（構成）。
    missing: usize,
    notes: Vec<String>,
    sizes_changing: bool,
    template_tip: String,
}

fn view(app: &AppState, win: &NpWindow) -> View {
    let lang = app.lang;
    let groups = win.groups(app);
    // 選んでいない・今のモデルが無いときは None（欄は空のまま。説明はツールチップ）。名前は表示の文字で判定しない
    let file_name = |path: &Path| {
        path.file_name().map_or_else(
            || path.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        )
    };
    let (model_name, model_tip) = match (&win.prep, win.model_path()) {
        (_, Some(path)) => (Some(file_name(path)), path.display().to_string()),
        // 今のモデルのファイルがあれば、そのファイル名（新規のウィンドウと同じ見え方）。なければ今のモデル（構成）・Live Link のモデル（新規）の名前
        _ => match (win.app_model(app), &app.np.model_file) {
            (_, Some(file)) if win.configure => (Some(file_name(file)), file.display().to_string()),
            (Some(m), _) => (Some(m.name.clone()), String::new()),
            _ => (None, String::new()),
        },
    };
    let missing = if win.configure && !groups.is_empty() {
        win.drafts
            .iter()
            .filter(|d| d.uid.is_some() && d.material.is_none())
            .count()
    } else {
        0
    };
    View {
        lang,
        model_name,
        model_tip,
        can_reload: win.configure
            && app.np.model_file.is_some()
            && !app.session.model.as_ref().is_some_and(|m| m.is_link()),
        link: win.configure && app.session.model.as_ref().is_some_and(|m| m.is_link()),
        unused: if win.configure {
            unused_groups(app, win)
        } else {
            0
        },
        missing,
        notes: win.notes().to_vec(),
        sizes_changing: win.drafts.iter().any(SetDraft::resizes),
        template_tip: win.template.tooltip(lang, &app.session.doc),
        groups,
    }
}

fn title(lang: Lang, configure: bool) -> &'static str {
    if configure {
        lang.t("newproject-project-configuration")
    } else {
        lang.t("newproject-new-project")
    }
}

/// 準備の様子の行（モデルを読んでいる・取り消した・読めなかった）があるか。
fn has_prep_row(win: &NpWindow) -> bool {
    matches!(
        win.prep,
        Prep::Loading { .. } | Prep::Failed { .. } | Prep::Canceled { .. }
    )
}

/// 状態の行（モデルに無いセット・セットの無いマテリアル・読み込みの知らせ）があるか。
fn has_summary_row(v: &View) -> bool {
    v.missing > 0 || v.unused > 0 || !v.notes.is_empty()
}

fn list_rows(win: &NpWindow, v: &View) -> usize {
    if win.configure {
        win.drafts.len().clamp(1, LIST_ROWS)
    } else {
        v.groups.len().min(LIST_ROWS)
    }
}

/// ウィンドウの高さ（描く順と同じ積み上げ）。
fn height(win: &NpWindow, v: &View) -> f32 {
    let mut h = window::HEADER_HEIGHT + PAD;
    if !win.configure {
        h += ROW + GAP; // テンプレート
    }
    h += ROW + GAP; // メッシュ
    if has_prep_row(win) {
        h += ROW + GAP;
    }
    if has_summary_row(v) {
        h += ROW + GAP;
    }
    if win.configure {
        h += 18.0 + list_rows(win, v) as f32 * SET_ROW + 4.0 + ROW + GAP;
    } else if !v.groups.is_empty() {
        h += 18.0 + list_rows(win, v) as f32 * SET_ROW + GAP;
    }
    h += ROW + GAP; // 解像度・補間方法
    h += ROW + GAP; // 法線の形式
    if !win.configure {
        h += ROW + GAP; // ベイク
    }
    h + 4.0 + FOOT
}

/// 毎フレーム: ウィンドウを描き、押されたものを `Action` として当てる。
pub fn show(ctx: &egui::Context, app: &mut AppState) {
    remove_confirm(ctx, app);
    let Some(mut win) = app.np.window.take() else {
        return;
    };
    let mut dropdown = app.ui.np_dropdown.take();
    let v = view(app, &win);
    let lang = v.lang;
    let configure = win.configure;
    // 文字を打っている間のキーは、その欄に任せる。確かめの一覧が出ているときは、そのウィンドウの取消・決定
    let keys_free = !ctx.egui_wants_keyboard_input() && dropdown.is_none();
    let (enter, esc) = if keys_free {
        ctx.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::NONE, Key::Enter),
                i.consume_key(egui::Modifiers::NONE, Key::Escape),
            )
        })
    } else {
        (false, false)
    };
    let spec = Spec {
        title: title(lang, configure),
        icon: Some(if configure { "tune" } else { "add" }),
        size: vec2(WIDTH, height(&win, &v)),
        modal: true,
        close_label: lang.t("newproject-close-window"),
    };
    let mut actions: Vec<NpAction> = Vec::new();
    let mut offset = win.offset;
    let locked_confirm = win.confirm.is_some();
    let closed = window::show(ctx, main_id(), &spec, &mut offset, false, |ui, frame| {
        draw(ui, frame, &mut win, &mut dropdown, &v, &mut actions);
    });
    win.offset = offset;
    // 一覧が開いていればその Esc は一覧の分（メニューが自分で見る）。確かめの一覧が出ていれば Esc は確かめをやめる
    if esc {
        actions.push(if locked_confirm {
            NpAction::ConfirmCancel
        } else {
            NpAction::Close
        });
    } else if enter {
        actions.push(if locked_confirm {
            NpAction::ConfirmApply
        } else {
            NpAction::Submit
        });
    }
    if closed {
        actions.push(if locked_confirm {
            NpAction::ConfirmCancel
        } else {
            NpAction::Close
        });
    }
    if win.is_loading() {
        ctx.request_repaint_after(std::time::Duration::from_millis(50));
    }
    dropdown_popup(ctx, &mut dropdown, &win, &v, &mut actions);
    app.np.window = Some(win);
    app.ui.np_dropdown = dropdown;
    for a in actions {
        app.np_apply(a);
    }
}

fn draw(
    ui: &mut Ui,
    frame: &Frame,
    win: &mut NpWindow,
    dropdown: &mut OpenDropdown,
    v: &View,
    actions: &mut Vec<NpAction>,
) {
    let lang = v.lang;
    let locked = win.confirm.is_some();
    let body = frame.body;
    let p = ui.painter().clone();
    let left = body.left() + PAD;
    let width = body.width() - 2.0 * PAD;
    let id = main_id();
    let mut y = body.top() + PAD;
    let label = |p: &egui::Painter, y: f32, text: &str, enabled: bool| {
        w::text(
            p,
            Rect::from_min_size(pos2(left, y), vec2(LABEL_W - 8.0, ROW)),
            text,
            t::LABEL.with_color(if enabled { t::TEXT } else { t::TEXT_DISABLED }),
            Align::Left,
        );
    };
    let control = |y: f32| Rect::from_min_max(pos2(left + LABEL_W, y), pos2(left + width, y + ROW));

    // テンプレート（新規）
    if !win.configure {
        label(&p, y, lang.t("newproject-template"), true);
        let (r, b) = w::dropdown(
            ui,
            control(y),
            id.with("template"),
            None,
            win.template.name(lang),
            Some(&v.template_tip),
            !locked,
            0.0,
        );
        if r.clicked() {
            open_dropdown(ui, dropdown, Dropdown::Template, b);
        }
        y += ROW + GAP;
    }

    // メッシュ
    label(&p, y, lang.t("newproject-mesh"), true);
    {
        let c = control(y);
        let mut x = c.right();
        let folder = Rect::from_min_size(pos2(x - 26.0, y), vec2(26.0, ROW));
        x -= 30.0;
        let choose_enabled = !locked && !v.link;
        if w::icon_button(
            ui,
            folder,
            id.with("choose"),
            "folder_open",
            lang.t("newproject-choose-a-model-tip"),
            false,
            choose_enabled,
            17.0,
        )
        .clicked()
        {
            actions.push(NpAction::ChooseDialog);
        }
        // 外す（新規でモデルを選んでいる）／替えるのをやめる（構成で替えようとしている）
        if win.model.is_some() {
            let clear = Rect::from_min_size(pos2(x - 26.0, y), vec2(26.0, ROW));
            x -= 30.0;
            if w::icon_button(
                ui,
                clear,
                id.with("clear"),
                "close",
                if win.configure {
                    lang.t("newproject-keep-the-current-model-tip")
                } else {
                    lang.t("newproject-do-not-use-a-model-tip")
                },
                false,
                !locked,
                15.0,
            )
            .clicked()
            {
                actions.push(NpAction::ClearModel);
            }
        }
        if v.can_reload {
            let reload = Rect::from_min_size(pos2(x - 26.0, y), vec2(26.0, ROW));
            x -= 30.0;
            if w::icon_button(
                ui,
                reload,
                id.with("reload"),
                "restart_alt",
                lang.t("newproject-reload-the-model-from-its-tip"),
                win.reload,
                !locked,
                17.0,
            )
            .clicked()
            {
                actions.push(NpAction::Reload);
            }
        }
        let box_rect = Rect::from_min_max(c.min, pos2(x, y + ROW));
        w::rounded(&p, box_rect, t::CONTROL_BG, 3.0);
        w::outline(&p, box_rect, t::BORDER, 1.0, 3.0);
        w::icon(
            &p,
            Rect::from_min_size(box_rect.min + vec2(4.0, 0.0), vec2(20.0, ROW)),
            "view_in_ar",
            t::TEXT_DIM,
            15.0,
        );
        let none = v.model_name.is_none();
        let name_rect = Rect::from_min_max(
            pos2(box_rect.left() + 28.0, y),
            pos2(box_rect.right() - 6.0, y + ROW),
        );
        if let Some(name) = &v.model_name {
            let shown = w::fit(&p, name, name_rect.width(), t::LABEL);
            w::text(&p, name_rect, &shown, t::LABEL, Align::Left);
        }
        let tip = if v.model_tip.is_empty() {
            if none {
                lang.t("newproject-paint-in-2d-a-model-tip").to_owned()
            } else {
                String::new()
            }
        } else {
            v.model_tip.clone()
        };
        if !tip.is_empty() {
            ui.interact(box_rect, id.with("model-box"), Sense::hover())
                .on_hover_text(tip);
        }
    }
    y += ROW + GAP;

    // 準備の様子
    if has_prep_row(win) {
        let row = Rect::from_min_size(pos2(left + LABEL_W, y), vec2(width - LABEL_W, ROW));
        match &win.prep {
            Prep::Loading { job, .. } => {
                let cancel = Rect::from_min_size(pos2(row.right() - 26.0, y), vec2(26.0, ROW));
                if w::icon_button(
                    ui,
                    cancel,
                    id.with("cancel-prep"),
                    "close",
                    lang.t("newproject-cancel-preparation-tip"),
                    false,
                    !locked,
                    15.0,
                )
                .clicked()
                {
                    actions.push(NpAction::CancelPrepare);
                }
                // 割合が分かれば帯を埋めて % を添え、分からない間は往復する帯
                let fraction = job.fraction();
                let label = lang.t("newproject-preparing-the-model");
                let label = match fraction {
                    Some(f) => format!("{label} {}%", (f * 100.0).floor() as u32),
                    None => label.to_owned(),
                };
                let text_rect = Rect::from_min_size(row.min, vec2(row.width() - 34.0, 14.0));
                w::text(&p, text_rect, &label, t::LABEL_DIM, Align::Left);
                let bar =
                    Rect::from_min_size(pos2(row.left(), y + 16.0), vec2(row.width() - 34.0, 5.0));
                w::progress_bar(&p, bar, fraction, ui.ctx().input(|i| i.time));
            }
            Prep::Canceled { .. } | Prep::Failed { .. } => {
                let again = lang.t("newproject-prepare-again");
                let bw = w::text_width(&p, again, t::LABEL) + 28.0;
                let button = Rect::from_min_size(pos2(row.right() - bw, y), vec2(bw, ROW));
                if w::button(
                    ui,
                    button,
                    id.with("again"),
                    again,
                    false,
                    !locked,
                    None,
                    None,
                )
                .clicked()
                {
                    actions.push(NpAction::PrepareAgain);
                }
                let (text, warning, tip) = match &win.prep {
                    Prep::Failed { error, .. } => {
                        let why = lang.view_error(error);
                        (why.clone(), true, why)
                    }
                    _ => (
                        lang.t("newproject-preparation-canceled").to_owned(),
                        false,
                        String::new(),
                    ),
                };
                let at = Rect::from_min_size(row.min, vec2(row.width() - bw - 8.0, ROW));
                if warning {
                    w::icon(
                        &p,
                        Rect::from_min_size(at.min, vec2(18.0, ROW)),
                        "warning",
                        t::WARNING,
                        15.0,
                    );
                }
                let text_rect = Rect::from_min_max(
                    pos2(at.left() + if warning { 22.0 } else { 0.0 }, y),
                    at.max,
                );
                let shown = w::fit(&p, &text, text_rect.width(), t::LABEL_DIM);
                w::text(
                    &p,
                    text_rect,
                    &shown,
                    t::LABEL_DIM.with_color(if warning { t::WARNING } else { t::TEXT_DIM }),
                    Align::Left,
                );
                if !tip.is_empty() {
                    ui.interact(at, id.with("prep-error"), Sense::hover())
                        .on_hover_text(tip);
                }
            }
            _ => {}
        }
        y += ROW + GAP;
    }

    // 状態（モデルに無いセット・セットの無いマテリアル・読み込みの知らせ）
    if has_summary_row(v) {
        let mut x = left + LABEL_W;
        let limit = left + width;
        let chip = |ui: &mut Ui,
                    x: &mut f32,
                    key: &str,
                    icon: &'static str,
                    color: egui::Color32,
                    text: &str,
                    tip: &str| {
            let tw = w::text_width(ui.painter(), text, t::LABEL_DIM);
            let r = Rect::from_min_size(pos2(*x, y), vec2((tw + 28.0).min(limit - *x), ROW));
            w::icon(
                ui.painter(),
                Rect::from_min_size(r.min, vec2(18.0, ROW)),
                icon,
                color,
                14.0,
            );
            w::text(
                ui.painter(),
                Rect::from_min_max(pos2(r.left() + 20.0, y), r.max),
                text,
                t::LABEL_DIM.with_color(color),
                Align::Left,
            );
            ui.interact(r, id.with(("chip", key)), Sense::hover())
                .on_hover_text(tip);
            *x += r.width() + 10.0;
        };
        if v.missing > 0 {
            let names: Vec<String> = win
                .drafts
                .iter()
                .filter(|d| d.uid.is_some() && d.material.is_none())
                .map(|d| d.name.clone())
                .collect();
            chip(
                ui,
                &mut x,
                "missing",
                "link_off",
                t::WARNING,
                &lang.f(
                    "newproject-not-in-this-model-tip",
                    &[("missing", &v.missing)],
                ),
                &names.join("\n"),
            );
        }
        if v.unused > 0 {
            let names: Vec<String> = v
                .groups
                .iter()
                .filter(|g| win.drafts.iter().all(|d| d.material != Some(g.index)))
                .map(|g| g.name.clone())
                .collect();
            let text = lang.f(
                "newproject-materials-without-a-texture-set-tip",
                &[("unused", &v.unused)],
            );
            let tw = w::text_width(ui.painter(), &text, t::LABEL_DIM);
            chip(
                ui,
                &mut x,
                "unused",
                "info",
                t::TEXT_DIM,
                &text,
                &names.join("\n"),
            );
            let add = Rect::from_min_size(pos2((x - 6.0).min(limit - 26.0), y), vec2(26.0, ROW));
            let _ = tw;
            if w::icon_button(
                ui,
                add,
                id.with("add-unused"),
                "add",
                lang.t("newproject-add-an-empty-texture-set-tip"),
                false,
                !locked,
                16.0,
            )
            .clicked()
            {
                actions.push(NpAction::AddUnused);
            }
            x += 30.0;
        }
        if !v.notes.is_empty() {
            let text = lang.f(
                "newproject-notices-while-loading-tip",
                &[("notes", &v.notes.len())],
            );
            chip(
                ui,
                &mut x,
                "notes",
                "warning",
                t::WARNING,
                &text,
                &v.notes.join("\n"),
            );
        }
        y += ROW + GAP;
    }

    // テクスチャセット
    if win.configure {
        draw_drafts(ui, win, dropdown, v, left, width, &mut y, locked, actions);
    } else if !v.groups.is_empty() {
        draw_materials(ui, win, v, left, width, &mut y, locked, actions);
    }

    // 解像度（新規）・補間方法（構成）
    if win.configure {
        label(&p, y, lang.t("newproject-resampling"), v.sizes_changing);
        let tip = lang.t("newproject-how-a-texture-set-is-tip");
        let c = control(y);
        let box_rect = Rect::from_min_max(
            c.min,
            pos2(
                c.right() - if v.sizes_changing { 28.0 } else { 0.0 },
                c.bottom(),
            ),
        );
        let (r, b) = w::dropdown(
            ui,
            box_rect,
            id.with("resampling"),
            None,
            resampling_name(lang, win.resampling),
            Some(tip),
            !locked && v.sizes_changing,
            0.0,
        );
        if r.clicked() {
            open_dropdown(ui, dropdown, Dropdown::Resampling, b);
        }
        if v.sizes_changing {
            let at = Rect::from_min_size(pos2(c.right() - 24.0, y), vec2(24.0, ROW));
            w::icon(&p, at, "warning", t::WARNING, 15.0);
            ui.interact(at, id.with("resample-warning"), Sense::hover())
                .on_hover_text(lang.t("newproject-applying-resamples-the-resized-texture-tip"));
        }
    } else {
        label(&p, y, lang.t("newproject-resolution"), true);
        let (r, b) = w::dropdown(
            ui,
            control(y),
            id.with("resolution"),
            None,
            &format!("{0} × {0}", win.resolution),
            None,
            !locked,
            0.0,
        );
        if r.clicked() {
            open_dropdown(ui, dropdown, Dropdown::Resolution, b);
        }
    }
    y += ROW + GAP;

    // 法線の形式
    label(&p, y, lang.t("newproject-normal-map-format"), true);
    let (r, b) = w::dropdown(
        ui,
        control(y),
        id.with("normal"),
        None,
        normal_name(lang, win.normal),
        Some(lang.t("newproject-unity-reads-normal-maps-as-tip")),
        !locked,
        0.0,
    );
    if r.clicked() {
        open_dropdown(ui, dropdown, Dropdown::Normal, b);
    }
    y += ROW + GAP;

    // 作成後のベイク（新規）
    if !win.configure {
        let can_bake = win.model.is_some() && matches!(win.prep, Prep::Ready { .. });
        let row = Rect::from_min_size(pos2(left + LABEL_W, y), vec2(width - LABEL_W, ROW));
        let next = w::toggle(
            ui,
            row,
            id.with("bake"),
            lang.t("newproject-bake-mesh-maps-after-creating"),
            win.bake && can_bake,
            Some(lang.t("newproject-normal-position-ao-curvature-and-tip")),
            can_bake && !locked,
        );
        if next != win.bake && can_bake {
            actions.push(NpAction::BakeAfter(next));
        }
    }

    // 下の帯: 決められない理由（左）と、取消・決定（右）
    let foot = Rect::from_min_max(pos2(body.left(), body.bottom() - FOOT), body.max);
    w::fill(&p, foot, t::PANEL_HEADER);
    w::hline(&p, foot.left(), foot.right(), foot.top(), t::BORDER);
    if let Some(error) = &win.error {
        let at = Rect::from_min_size(
            pos2(foot.left() + PAD, foot.top()),
            vec2(foot.width() - 2.0 * PAD - 232.0, foot.height()),
        );
        w::icon(
            &p,
            Rect::from_min_size(at.min, vec2(18.0, at.height())),
            "warning",
            t::WARNING,
            15.0,
        );
        let text_rect = Rect::from_min_max(pos2(at.left() + 22.0, at.top()), at.max);
        let shown = w::fit(&p, error, text_rect.width(), t::LABEL);
        w::text(
            &p,
            text_rect,
            &shown,
            t::LABEL.with_color(t::WARNING),
            Align::Left,
        );
        ui.interact(at, id.with("error"), Sense::hover())
            .on_hover_text(error.as_str());
    }
    let cancel = Rect::from_min_size(
        pos2(foot.right() - PAD - 224.0, foot.top() + 12.0),
        vec2(104.0, 28.0),
    );
    let ok = Rect::from_min_size(
        pos2(foot.right() - PAD - 112.0, foot.top() + 12.0),
        vec2(112.0, 28.0),
    );
    if w::button(
        ui,
        cancel,
        id.with("cancel"),
        lang.t("newproject-cancel"),
        false,
        !locked,
        None,
        None,
    )
    .clicked()
    {
        actions.push(NpAction::Close);
    }
    let ready = win.is_ready();
    let tip = (!ready).then(|| {
        if win.is_loading() {
            lang.t("newproject-the-model-is-being-prepared-tip")
        } else {
            lang.t("newproject-the-model-is-not-ready-tip")
        }
    });
    if w::button(
        ui,
        ok,
        id.with("ok"),
        if win.configure {
            lang.t("newproject-apply")
        } else {
            lang.t("newproject-create")
        },
        true,
        ready && !locked,
        tip,
        None,
    )
    .clicked()
    {
        actions.push(NpAction::Submit);
    }

    // 確かめ（ウィンドウの上に重ねる。下の部品へは通さない）
    if let Some(plan) = win.confirm.clone() {
        draw_confirm(ui, frame, &plan, lang, actions);
    }
}

/// ウィンドウの中の、スクロールする一覧の枠（`area` の中にだけ描く）。返すのは、一覧の中に描く Ui と、スクロールの寸法（描き終えたあとの
/// `Scroll::end` がつまみを出す）。
fn list_frame(ui: &mut Ui, area: Rect, scroll: &mut f32, content: f32) -> (Ui, Scroll) {
    let bar = Scroll::begin(ui, area, content, scroll);
    let mut child = ui.new_child(UiBuilder::new().max_rect(area));
    child.set_clip_rect(area.intersect(ui.clip_rect()));
    (child, bar)
}

/// 新規: テクスチャセットにするマテリアルのチェックの一覧（既定は全部。最後の 1 つは外せない）。使うメッシュの名前を右に添える。
#[allow(clippy::too_many_arguments)]
fn draw_materials(
    ui: &mut Ui,
    win: &mut NpWindow,
    v: &View,
    left: f32,
    width: f32,
    y: &mut f32,
    locked: bool,
    actions: &mut Vec<NpAction>,
) {
    let lang = v.lang;
    let id = main_id();
    let p = ui.painter().clone();
    w::text(
        &p,
        Rect::from_min_size(pos2(left, *y), vec2(width, 16.0)),
        lang.t("newproject-texture-sets"),
        t::HEADER.with_color(t::TEXT_DIM),
        Align::Left,
    );
    *y += 18.0;
    let visible = v.groups.len().min(LIST_ROWS);
    let area = Rect::from_min_size(pos2(left, *y), vec2(width, visible as f32 * SET_ROW));
    let content = v.groups.len() as f32 * SET_ROW;
    let (mut child, bar) = list_frame(ui, area, &mut win.list_scroll, content);
    let cp = child.painter().clone();
    let chosen = win.chosen(v.groups.len());
    let capped = chosen.len() >= MAX_SETS;
    for (i, g) in v.groups.iter().enumerate() {
        let row = Rect::from_min_size(
            pos2(
                area.left(),
                area.top() + i as f32 * SET_ROW - win.list_scroll,
            ),
            vec2(area.width() - bar.reserved(), SET_ROW - 2.0),
        );
        if row.bottom() < area.top() || row.top() > area.bottom() {
            continue;
        }
        let on = chosen.contains(&g.index);
        let last = on && chosen.len() == 1;
        // 上限まで選んでいれば、ほかは選べない（理由はツールチップ）
        let full = !on && capped;
        let name_w = (row.width() * 0.5).min(w::text_width(&cp, &g.name, t::LABEL) + 24.0);
        let detail = format!(
            "{}\n{}: {}\n{}",
            g.name,
            lang.t("newproject-meshes"),
            g.meshes.join(", "),
            lang.f("newproject-material-slot-s", &[("slots", &g.slots)]),
        );
        let tip = if last {
            format!(
                "{}\n{detail}",
                lang.t("newproject-at-least-one-stays-checked-tip")
            )
        } else if full {
            format!("{}\n{detail}", crate::lang::refusals::set_limit(lang))
        } else {
            detail
        };
        let next = w::toggle(
            &mut child,
            Rect::from_min_size(row.min, vec2(name_w, row.height())),
            id.with(("material", i)),
            &g.name,
            on,
            Some(&tip),
            !last && !full && !locked,
        );
        if next != on {
            actions.push(NpAction::Material(g.index, next));
        }
        let meshes = g.meshes.join(", ");
        let at = Rect::from_min_max(pos2(row.left() + name_w + 12.0, row.top()), row.max);
        let shown = w::fit(&cp, &meshes, at.width(), t::LABEL_SMALL);
        w::text(&cp, at, &shown, t::LABEL_SMALL, Align::Left);
    }
    bar.end(ui, (id, "list-scroll"), &mut win.list_scroll);
    *y += area.height() + GAP;
}

/// 構成: セットの並び（名前・大きさ・マテリアル・消す）と「足す」。
#[allow(clippy::too_many_arguments)]
fn draw_drafts(
    ui: &mut Ui,
    win: &mut NpWindow,
    dropdown: &mut OpenDropdown,
    v: &View,
    left: f32,
    width: f32,
    y: &mut f32,
    locked: bool,
    actions: &mut Vec<NpAction>,
) {
    let lang = v.lang;
    let id = main_id();
    let p = ui.painter().clone();
    w::text(
        &p,
        Rect::from_min_size(pos2(left, *y), vec2(width, 16.0)),
        lang.t("newproject-texture-sets"),
        t::HEADER.with_color(t::TEXT_DIM),
        Align::Left,
    );
    *y += 18.0;
    let visible = list_rows(win, v);
    let area = Rect::from_min_size(pos2(left, *y), vec2(width, visible as f32 * SET_ROW));
    let content = win.drafts.len() as f32 * SET_ROW;
    let (mut child, bar) = list_frame(ui, area, &mut win.list_scroll, content);
    let cp = child.painter().clone();
    let count = win.drafts.len();
    let drafts = win.drafts.clone();
    for (i, d) in drafts.iter().enumerate() {
        let row = Rect::from_min_size(
            pos2(
                area.left(),
                area.top() + i as f32 * SET_ROW - win.list_scroll,
            ),
            vec2(area.width() - bar.reserved(), SET_ROW - 2.0),
        );
        if row.bottom() < area.top() || row.top() > area.bottom() {
            continue;
        }
        let remove = Rect::from_min_size(
            pos2(row.right() - 24.0, row.top()),
            vec2(24.0, row.height()),
        );
        let material = Rect::from_min_size(
            pos2(remove.left() - 4.0 - 156.0, row.top()),
            vec2(156.0, row.height()),
        );
        let size = Rect::from_min_size(
            pos2(material.left() - 4.0 - 88.0, row.top()),
            vec2(88.0, row.height()),
        );
        let name = Rect::from_min_max(row.min, pos2(size.left() - 6.0, row.bottom()));
        if d.read_only {
            let shown = w::fit(&cp, &d.name, name.width() - 24.0, t::LABEL);
            w::icon(
                &cp,
                Rect::from_min_size(name.min, vec2(20.0, name.height())),
                "lock",
                t::WARNING,
                14.0,
            );
            w::text(
                &cp,
                Rect::from_min_max(pos2(name.left() + 22.0, name.top()), name.max),
                &shown,
                t::LABEL.with_color(t::TEXT_DIM),
                Align::Left,
            );
            child
                .interact(name, id.with(("name-ro", i)), Sense::hover())
                .on_hover_text(lang.t("newproject-read-only-texture-set-tip"));
        } else {
            let out = w::text_field(
                &mut child,
                name,
                id.with(("name", i)),
                &d.name,
                Some(if d.uid.is_none() {
                    lang.t("newproject-a-new-empty-texture-set-tip")
                } else {
                    lang.t("newproject-texture-set-name-tip")
                }),
                false,
            );
            if let Some(next) = out.committed {
                actions.push(NpAction::Draft(i, DraftOp::Name(next)));
            }
        }
        // 大きさ
        let shown_size = format!(
            "{}{}",
            if d.resizes() { "→ " } else { "" },
            size_text(d.size)
        );
        let size_tip = if d.uid.is_none() {
            lang.t("newproject-size-of-the-new-texture-tip").to_owned()
        } else {
            lang.f(
                "newproject-size-of-this-texture-set-tip",
                &[("current", &d.current.0), ("current_2", &d.current.1)],
            )
        };
        let (r, b) = w::dropdown(
            &mut child,
            size,
            id.with(("size", i)),
            None,
            &shown_size,
            Some(&size_tip),
            !locked && !d.read_only,
            0.0,
        );
        if r.clicked() {
            open_dropdown(&child, dropdown, Dropdown::DraftSize(i), b);
        }
        // マテリアル
        let (shown_material, material_tip) = match d.material.and_then(|g| v.groups.get(g)) {
            Some(g) => (
                g.name.clone(),
                format!(
                    "{}\n{}: {}",
                    lang.t("newproject-the-material-this-texture-set"),
                    lang.t("newproject-meshes"),
                    g.meshes.join(", ")
                ),
            ),
            None => (
                if v.groups.is_empty() {
                    "—".to_owned()
                } else {
                    lang.t("newproject-not-in-this-model").to_owned()
                },
                lang.t("newproject-the-material-this-texture-set-2")
                    .to_owned(),
            ),
        };
        let (r, b) = w::dropdown(
            &mut child,
            material,
            id.with(("material", i)),
            None,
            &shown_material,
            Some(&material_tip),
            !locked && !v.groups.is_empty(),
            0.0,
        );
        if r.clicked() {
            open_dropdown(&child, dropdown, Dropdown::DraftMaterial(i), b);
        }
        // 消す
        let last = count <= 1;
        if w::icon_button(
            &mut child,
            remove,
            id.with(("remove", i)),
            "delete",
            if last {
                lang.t("newproject-a-project-keeps-at-least-tip")
            } else {
                lang.t("newproject-remove-this-texture-set-asked-tip")
            },
            false,
            !last && !locked,
            15.0,
        )
        .clicked()
        {
            actions.push(NpAction::Draft(i, DraftOp::Remove));
        }
    }
    bar.end(ui, (id, "list-scroll"), &mut win.list_scroll);
    *y += area.height() + 4.0;
    let add = Rect::from_min_size(pos2(left, *y), vec2(width.min(240.0), ROW));
    let can_add = win.drafts.len() < super::MAX_SETS;
    if w::button(
        ui,
        add,
        id.with("add"),
        lang.t("newproject-add-texture-set"),
        false,
        can_add && !locked,
        Some(lang.t("newproject-an-empty-texture-set-for-tip")),
        Some("add"),
    )
    .clicked()
    {
        actions.push(NpAction::AddDraft);
    }
    *y += ROW + GAP;
}

fn open_dropdown(ui: &Ui, dropdown: &mut OpenDropdown, kind: Dropdown, anchor: Rect) {
    *dropdown = Some((
        kind,
        PopupState::new(ui.ctx(), anchor).with_min_width(anchor.width()),
    ));
}

pub fn normal_name(lang: Lang, direction: NormalYDirection) -> &'static str {
    match direction {
        NormalYDirection::OpenGL => lang.t("newproject-opengl-y-unity"),
        NormalYDirection::DirectX => lang.t("newproject-directx-y"),
    }
}

/// 開いているドロップダウンの項目。
fn dropdown_entries(win: &NpWindow, v: &View, kind: Dropdown) -> Vec<Entry<NpAction>> {
    let lang = v.lang;
    match kind {
        Dropdown::Template => Template::ALL
            .iter()
            .map(|t| Entry::item(t.name(lang), NpAction::Template(*t)).radio(*t == win.template))
            .collect(),
        Dropdown::Resolution => RESOLUTIONS
            .iter()
            .map(|r| {
                Entry::item(format!("{r} × {r}"), NpAction::Resolution(*r))
                    .radio(*r == win.resolution)
            })
            .collect(),
        Dropdown::Normal => [NormalYDirection::OpenGL, NormalYDirection::DirectX]
            .iter()
            .map(|n| {
                Entry::item(normal_name(lang, *n), NpAction::Normal(*n)).radio(*n == win.normal)
            })
            .collect(),
        Dropdown::Resampling => [
            None,
            Some(CanvasResampling::Bilinear),
            Some(CanvasResampling::Area),
            Some(CanvasResampling::Nearest),
        ]
        .iter()
        .map(|m| {
            Entry::item(resampling_name(lang, *m), NpAction::Resampling(*m))
                .radio(*m == win.resampling)
        })
        .collect(),
        Dropdown::DraftSize(i) => {
            let Some(d) = win.drafts.get(i) else {
                return Vec::new();
            };
            let mut entries = Vec::new();
            let standard = d.current.0 == d.current.1 && RESOLUTIONS.contains(&d.current.0);
            let current = lang.t("newproject-current");
            if d.uid.is_some() && !standard {
                entries.push(
                    Entry::item(
                        format!("{} × {} ({current})", d.current.0, d.current.1),
                        NpAction::Draft(i, DraftOp::Size(d.current.0, d.current.1)),
                    )
                    .radio(d.size == d.current),
                );
            }
            for r in RESOLUTIONS {
                let label = if d.uid.is_some() && d.current == (r, r) {
                    format!("{r} × {r} ({current})")
                } else {
                    format!("{r} × {r}")
                };
                entries.push(
                    Entry::item(label, NpAction::Draft(i, DraftOp::Size(r, r)))
                        .radio(d.size == (r, r)),
                );
            }
            entries
        }
        Dropdown::DraftMaterial(i) => v
            .groups
            .iter()
            .map(|g| {
                let taken = win
                    .drafts
                    .iter()
                    .enumerate()
                    .any(|(j, d)| j != i && d.material == Some(g.index));
                let label = if g.meshes.is_empty() {
                    g.name.clone()
                } else {
                    format!("{}  —  {}", g.name, g.meshes.join(", "))
                };
                Entry::item(label, NpAction::Draft(i, DraftOp::Material(Some(g.index))))
                    .radio(
                        win.drafts
                            .get(i)
                            .is_some_and(|d| d.material == Some(g.index)),
                    )
                    .enabled(!taken)
            })
            .collect(),
    }
}

fn dropdown_popup(
    ctx: &egui::Context,
    dropdown: &mut OpenDropdown,
    win: &NpWindow,
    v: &View,
    actions: &mut Vec<NpAction>,
) {
    let Some((kind, mut state)) = dropdown.take() else {
        return;
    };
    let entries = dropdown_entries(win, v, kind);
    match menu::show(
        ctx,
        Id::new("yolu.newproject.dropdown"),
        &mut state,
        &entries,
        &[],
    ) {
        PopupOutcome::Open | PopupOutcome::Step(_) => *dropdown = Some((kind, state)),
        PopupOutcome::Chosen(action) => actions.push(action),
        PopupOutcome::Close => {}
    }
}

/// 確かめの題（何を確かめるか）。
fn confirm_title(lang: Lang, plan: &Plan, reload: bool) -> &'static str {
    match plan.kinds.as_slice() {
        [ConfirmKind::Removal] => lang.t("newproject-remove-texture-sets"),
        [ConfirmKind::Resize] => lang.t("newproject-resize-texture-sets"),
        [ConfirmKind::Model] if reload => lang.t("newproject-reload-the-model"),
        [ConfirmKind::Model] => lang.t("newproject-change-the-model"),
        _ => lang.t("newproject-apply-the-configuration"),
    }
}

/// 適用の前の確かめ: ウィンドウの上に重ねる一覧（消す・大きさ・モデル）。取り消せない理由は 1 行の警告だけ。
fn draw_confirm(ui: &mut Ui, frame: &Frame, plan: &Plan, lang: Lang, actions: &mut Vec<NpAction>) {
    let id = main_id().with("confirm");
    let p = ui.painter().clone();
    let rect = frame.rect;
    // 下の部品へ通さない（同じレイヤーで後から足した部品が上になる）
    ui.interact(rect, id.with("blocker"), Sense::click_and_drag());
    w::rounded(&p, rect, egui::Color32::from_black_alpha(150), 6.0);
    let shown = plan.rows.len().min(CARD_ROWS);
    let more = plan.rows.len() > CARD_ROWS;
    let row_h = 22.0;
    let card_h = 40.0 + 30.0 + (shown + usize::from(more)) as f32 * row_h + 14.0 + 52.0;
    let card = Rect::from_center_size(
        rect.center(),
        vec2(rect.width() - 56.0, card_h.min(rect.height() - 24.0)),
    );
    w::rounded(&p, card, t::PANEL_BG, 6.0);
    w::outline(&p, card, t::SEPARATOR, 1.0, 6.0);
    let reload = plan
        .rows
        .iter()
        .any(|r| r.right == lang.t("newproject-reload"));
    w::icon(
        &p,
        Rect::from_min_size(card.min + vec2(14.0, 0.0), vec2(20.0, 40.0)),
        "warning",
        t::WARNING,
        17.0,
    );
    w::text(
        &p,
        Rect::from_min_size(card.min + vec2(40.0, 0.0), vec2(card.width() - 54.0, 40.0)),
        confirm_title(lang, plan, reload),
        t::HEADER,
        Align::Left,
    );
    w::text(
        &p,
        Rect::from_min_size(card.min + vec2(14.0, 40.0), vec2(card.width() - 28.0, 22.0)),
        lang.t("newproject-this-cannot-be-undone"),
        t::LABEL.with_color(t::WARNING),
        Align::Left,
    );
    let list_top = card.top() + 70.0;
    let right_w = plan
        .rows
        .iter()
        .map(|r| w::text_width(&p, &r.right, t::LABEL_DIM))
        .fold(0.0f32, f32::max);
    let mid_w = plan
        .rows
        .iter()
        .map(|r| w::text_width(&p, &r.middle, t::LABEL_DIM))
        .fold(0.0f32, f32::max);
    for (i, row) in plan.rows.iter().take(CARD_ROWS).enumerate() {
        let r = Rect::from_min_size(
            pos2(card.left() + 14.0, list_top + i as f32 * row_h),
            vec2(card.width() - 28.0, row_h),
        );
        let right = Rect::from_min_size(pos2(r.right() - right_w, r.top()), vec2(right_w, row_h));
        let mid_right = if right_w > 0.0 {
            right.left() - 12.0
        } else {
            r.right()
        };
        let middle = Rect::from_min_size(pos2(mid_right - mid_w, r.top()), vec2(mid_w, row_h));
        let left_right = if mid_w > 0.0 {
            middle.left() - 12.0
        } else {
            mid_right
        };
        let left = Rect::from_min_max(r.min, pos2(left_right, r.bottom()));
        let color = if row.warning { t::WARNING } else { t::TEXT };
        let text = w::fit(&p, &row.left, left.width(), t::LABEL);
        w::text(&p, left, &text, t::LABEL.with_color(color), Align::Left);
        w::text(&p, middle, &row.middle, t::LABEL_DIM, Align::Left);
        w::text(&p, right, &row.right, t::LABEL_DIM, Align::Right);
    }
    if more {
        let r = Rect::from_min_size(
            pos2(card.left() + 14.0, list_top + CARD_ROWS as f32 * row_h),
            vec2(card.width() - 28.0, row_h),
        );
        w::text(
            &p,
            r,
            &lang.f(
                "newproject-and-more",
                &[("card_rows", &(plan.rows.len() - CARD_ROWS))],
            ),
            t::LABEL_DIM,
            Align::Left,
        );
    }
    let foot_top = card.bottom() - 52.0;
    w::hline(&p, card.left(), card.right(), foot_top, t::BORDER);
    let ok = Rect::from_min_size(
        pos2(card.right() - 14.0 - 112.0, foot_top + 12.0),
        vec2(112.0, 28.0),
    );
    let cancel = Rect::from_min_size(
        pos2(ok.left() - 8.0 - 104.0, foot_top + 12.0),
        vec2(104.0, 28.0),
    );
    if w::button(
        ui,
        cancel,
        id.with("cancel"),
        lang.t("newproject-cancel-2"),
        false,
        true,
        None,
        None,
    )
    .clicked()
    {
        actions.push(NpAction::ConfirmCancel);
    }
    if w::button(
        ui,
        ok,
        id.with("apply"),
        lang.t("newproject-apply"),
        true,
        true,
        None,
        None,
    )
    .clicked()
    {
        actions.push(NpAction::ConfirmApply);
    }
}

/// テクスチャセットのパネルで「消す」を押したときの確認のウィンドウ（モーダル）。
fn remove_confirm(ctx: &egui::Context, app: &mut AppState) {
    let Some(uids) = app.np.remove_confirm.clone() else {
        return;
    };
    let lang = app.lang;
    let rows: Vec<crate::windows::Row> = uids
        .iter()
        .filter_map(|uid| {
            let i = app.session.sets.index_of(*uid)?;
            let set = app.session.sets.get(i)?;
            let doc = app.set_doc(i);
            Some(crate::windows::Row {
                left: set.name.clone(),
                middle: size_text((doc.width(), doc.height())),
                right: String::new(),
                warning: false,
            })
        })
        .collect();
    let spec = crate::windows::ListSpec {
        id: "remove-sets",
        title: if uids.len() == 1 {
            lang.t("newproject-remove-texture-set")
        } else {
            lang.t("newproject-remove-texture-sets")
        }
        .into(),
        icon: "warning",
        modal: true,
        width: 460.0,
        summary: Some((lang.t("newproject-this-cannot-be-undone").into(), true)),
        rows,
        buttons: vec![
            crate::windows::Button {
                label: lang.t("newproject-cancel-2").into(),
                primary: false,
                tooltip: None,
            },
            crate::windows::Button {
                label: lang.t("newproject-remove").into(),
                primary: true,
                tooltip: Some(lang.t("newproject-its-layers-history-and-baked-tip").into()),
            },
        ],
        close_label: lang.t("newproject-close-window").into(),
    };
    let mut offset: Vec2 = app.ui.np_remove_offset;
    let mut scroll = 0.0;
    let reply = crate::windows::show_list(ctx, &spec, &mut offset, &mut scroll);
    app.ui.np_remove_offset = offset;
    match reply {
        Some(crate::windows::Reply::Button(1)) => app.np_apply(NpAction::ConfirmRemove),
        Some(_) => app.np_apply(NpAction::CancelRemove),
        None => {}
    }
}
