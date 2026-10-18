//! 新しいプロジェクトのウィンドウで、作るときに複数のマテリアルを 1 つのテクスチャセットにまとめる案の下書きが共通で使う描き方。
//! 寸法と並びは本物のウィンドウ（`newproject/window.rs`）に合わせ、部品は `yolu_app::ui` の物だけ。

use egui::{pos2, vec2, Id, Painter, Rect, Ui, Vec2};
use yolu_app::ui::menu::{self, Entry as MenuEntry, PopupOutcome, PopupState};
use yolu_app::ui::theme as t;
use yolu_app::ui::widgets::{self as w, Align};
use yolu_app::ui::window::{self, Frame, Spec};

use super::multimat_common::{material_label, MATERIALS};
use crate::page::SketchCtx;

const WIDTH: f32 = 560.0;
const PAD: f32 = 16.0;
const ROW: f32 = 24.0;
const GAP: f32 = 8.0;
const LABEL_W: f32 = 168.0;
const FOOT: f32 = 52.0;
const SET_ROW: f32 = 26.0;
const MATERIAL_W: f32 = 156.0;
const SIZE_W: f32 = 88.0;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// 案 B: プロジェクト設定と同じセットの行。
    B,
    /// 案 B: 1 行目のマテリアルの選びを開いた形。
    BMenu,
}

impl Mode {
    fn open(self) -> bool {
        self == Mode::BMenu
    }
}

/// 案 B のセット（MATERIALS の添え字）。Tongue はどのセットにも入っていない（その行は消した）。
struct BSet {
    name: &'static str,
    materials: &'static [usize],
}

const B_SETS: [BSet; 4] = [
    BSet {
        name: "Face",
        materials: &[0, 1],
    },
    BSet {
        name: "Ear",
        materials: &[],
    },
    BSet {
        name: "Eye",
        materials: &[2],
    },
    BSet {
        name: "Body",
        materials: &[4, 5, 6],
    },
];

fn b_owner(m: usize) -> Option<&'static str> {
    B_SETS
        .iter()
        .find(|s| s.materials.contains(&m))
        .map(|s| s.name)
}

fn height() -> f32 {
    let mut h = window::HEADER_HEIGHT + PAD;
    h += 2.0 * (ROW + GAP); // テンプレート・メッシュ
    h += ROW + GAP; // セットの無いマテリアル
    h += 18.0 + B_SETS.len() as f32 * SET_ROW + 4.0 + ROW + GAP;
    h += 2.0 * (ROW + GAP); // 法線の形式・ベイク
    h + 4.0 + FOOT
}

/// 「新規プロジェクト」のウィンドウを画面の真ん中に描く。
pub fn new_window(ui: &mut Ui, ctx: &mut SketchCtx, mode: Mode) {
    let spec = Spec {
        title: ctx.pick("新規プロジェクト", "New Project"),
        icon: Some("add"),
        size: vec2(WIDTH, height()),
        modal: false,
        close_label: ctx.pick("ウィンドウを閉じる", "Close Window"),
    };
    let mut offset = Vec2::ZERO;
    let mut anchor: Option<Rect> = None;
    window::show(
        ui.ctx(),
        Id::new("multimat.new.window"),
        &spec,
        &mut offset,
        false,
        |ui, frame| anchor = body(ui, frame, ctx),
    );
    if let (true, Some(anchor)) = (mode.open(), anchor) {
        popup(ui, ctx, anchor);
    }
}

fn label(p: &Painter, left: f32, y: f32, text: &str) {
    w::text(
        p,
        Rect::from_min_size(pos2(left, y), vec2(LABEL_W - 8.0, ROW)),
        text,
        t::LABEL,
        Align::Left,
    );
}

fn body(ui: &mut Ui, frame: &Frame, ctx: &mut SketchCtx) -> Option<Rect> {
    let body = frame.body;
    let p = ui.painter().clone();
    let left = body.left() + PAD;
    let width = body.width() - 2.0 * PAD;
    let mut y = body.top() + PAD;
    let control = |y: f32| Rect::from_min_max(pos2(left + LABEL_W, y), pos2(left + width, y + ROW));

    // テンプレート
    label(&p, left, y, ctx.pick("テンプレート", "Template"));
    let _ = w::dropdown(
        ui,
        control(y),
        "mmn.template",
        None,
        ctx.pick("PBR（すべてのチャンネル）", "PBR (all channels)"),
        None,
        true,
        0.0,
    );
    y += ROW + GAP;

    // メッシュ
    label(&p, left, y, ctx.pick("メッシュ", "Mesh"));
    {
        let c = control(y);
        let folder = Rect::from_min_size(pos2(c.right() - 26.0, y), vec2(26.0, ROW));
        let _ = w::icon_button(
            ui,
            folder,
            "mmn.choose",
            "folder_open",
            ctx.pick("モデルを選ぶ", "Choose a Model"),
            false,
            true,
            17.0,
        );
        let clear = Rect::from_min_size(pos2(c.right() - 56.0, y), vec2(26.0, ROW));
        let _ = w::icon_button(
            ui,
            clear,
            "mmn.clear",
            "close",
            ctx.pick("モデルを使わない", "Do Not Use a Model"),
            false,
            true,
            15.0,
        );
        let box_rect = Rect::from_min_max(c.min, pos2(c.right() - 60.0, y + ROW));
        w::rounded(&p, box_rect, t::CONTROL_BG, 3.0);
        w::outline(&p, box_rect, t::BORDER, 1.0, 3.0);
        w::icon(
            &p,
            Rect::from_min_size(box_rect.min + vec2(4.0, 0.0), vec2(20.0, ROW)),
            "view_in_ar",
            t::TEXT_DIM,
            15.0,
        );
        w::text(
            &p,
            Rect::from_min_max(
                pos2(box_rect.left() + 28.0, y),
                pos2(box_rect.right() - 6.0, y + ROW),
            ),
            "Model.fbx",
            t::LABEL,
            Align::Left,
        );
    }
    y += ROW + GAP;

    let anchor = b_list(ui, ctx, left, width, &mut y);

    // 法線の形式
    label(
        &p,
        left,
        y,
        ctx.pick("ノーマルマップの形式", "Normal map format"),
    );
    let _ = w::dropdown(
        ui,
        control(y),
        "mmn.normal",
        None,
        ctx.pick("OpenGL（Y+、Unity）", "OpenGL (Y+, Unity)"),
        None,
        true,
        0.0,
    );
    y += ROW + GAP;
    // 作成後のベイク
    let row = Rect::from_min_size(pos2(left + LABEL_W, y), vec2(width - LABEL_W, ROW));
    let _ = w::toggle(
        ui,
        row,
        "mmn.bake",
        ctx.pick(
            "作成後にメッシュマップをベイクする",
            "Bake mesh maps after creating",
        ),
        false,
        None,
        true,
    );

    // 下の帯
    let foot = Rect::from_min_max(pos2(body.left(), body.bottom() - FOOT), body.max);
    w::fill(&p, foot, t::PANEL_HEADER);
    w::hline(&p, foot.left(), foot.right(), foot.top(), t::BORDER);
    let cancel = Rect::from_min_size(
        pos2(foot.right() - PAD - 224.0, foot.top() + 12.0),
        vec2(104.0, 28.0),
    );
    let ok = Rect::from_min_size(
        pos2(foot.right() - PAD - 112.0, foot.top() + 12.0),
        vec2(112.0, 28.0),
    );
    let _ = w::button(
        ui,
        cancel,
        "mmn.cancel",
        ctx.pick("キャンセル", "Cancel"),
        false,
        true,
        None,
        None,
    );
    let _ = w::button(
        ui,
        ok,
        "mmn.ok",
        ctx.pick("作成", "Create"),
        true,
        true,
        None,
        None,
    );
    anchor
}

fn heading(p: &Painter, ctx: &SketchCtx, left: f32, width: f32, y: f32) {
    w::text(
        p,
        Rect::from_min_size(pos2(left, y), vec2(width, 16.0)),
        ctx.pick("テクスチャセット", "Texture Sets"),
        t::HEADER.with_color(t::TEXT_DIM),
        Align::Left,
    );
}

/// 案 B: プロジェクト設定と同じ行（名前・大きさ・マテリアル・消す）と、セットの無いマテリアルの数・追加のボタン。最初の行のマテリアルの箱を返す。
fn b_list(ui: &mut Ui, ctx: &mut SketchCtx, left: f32, width: f32, y: &mut f32) -> Option<Rect> {
    let lang = ctx.lang;
    let p = ui.painter().clone();
    // セットの無いマテリアルの数（プロジェクト設定と同じ形: 印と文字、右に空のセットを追加する）
    {
        let text = ctx.pick(
            "セットの無いマテリアル 1",
            "1 material without a texture set",
        );
        let tw = w::text_width(&p, text, t::LABEL_DIM);
        let r = Rect::from_min_size(pos2(left + LABEL_W, *y), vec2(tw + 28.0, ROW));
        w::icon(
            &p,
            Rect::from_min_size(r.min, vec2(18.0, ROW)),
            "info",
            t::TEXT_DIM,
            14.0,
        );
        w::text(
            &p,
            Rect::from_min_max(pos2(r.left() + 20.0, *y), r.max),
            text,
            t::LABEL_DIM.with_color(t::TEXT_DIM),
            Align::Left,
        );
        let add = Rect::from_min_size(pos2(r.right() - 6.0, *y), vec2(26.0, ROW));
        let _ = w::icon_button(
            ui,
            add,
            "mmn.b.add-unused",
            "add",
            ctx.pick(
                "セットの無いマテリアルに、空のテクスチャセットを追加",
                "Add an empty texture set for each material without one",
            ),
            false,
            true,
            16.0,
        );
        *y += ROW + GAP;
    }
    heading(&p, ctx, left, width, *y);
    *y += 18.0;
    let mut first = None;
    for (i, set) in B_SETS.iter().enumerate() {
        let row = Rect::from_min_size(
            pos2(left, *y + i as f32 * SET_ROW),
            vec2(width, SET_ROW - 2.0),
        );
        let remove = Rect::from_min_size(
            pos2(row.right() - 24.0, row.top()),
            vec2(24.0, row.height()),
        );
        let material = Rect::from_min_size(
            pos2(remove.left() - 4.0 - MATERIAL_W, row.top()),
            vec2(MATERIAL_W, row.height()),
        );
        let size = Rect::from_min_size(
            pos2(material.left() - 4.0 - SIZE_W, row.top()),
            vec2(SIZE_W, row.height()),
        );
        let name = Rect::from_min_max(row.min, pos2(size.left() - 6.0, row.bottom()));
        let _ = w::text_field(ui, name, ("mmn.b.name", i), set.name, None, false);
        let _ = w::dropdown(
            ui,
            size,
            ("mmn.b.size", i),
            None,
            "2048 × 2048",
            None,
            true,
            0.0,
        );
        let names: Vec<&str> = set.materials.iter().map(|&m| MATERIALS[m]).collect();
        let shown = material_label(&p, lang, &names, material.width() - 26.0);
        let (_, b) = w::dropdown(
            ui,
            material,
            ("mmn.b.material", i),
            None,
            &shown,
            None,
            true,
            0.0,
        );
        if set.materials.is_empty() {
            w::icon(
                &p,
                Rect::from_min_size(b.min + vec2(5.0, 0.0), vec2(18.0, b.height())),
                "warning",
                t::WARNING,
                14.0,
            );
            w::text(
                &p,
                Rect::from_min_max(
                    pos2(b.left() + 25.0, b.top()),
                    pos2(b.right() - 22.0, b.bottom()),
                ),
                ctx.pick("マテリアルなし", "No material"),
                t::LABEL.with_color(t::WARNING),
                Align::Left,
            );
        }
        if i == 0 {
            first = Some(b);
        }
        let _ = w::icon_button(
            ui,
            remove,
            ("mmn.b.remove", i),
            "delete",
            ctx.pick("このテクスチャセットを消す", "Remove this texture set"),
            false,
            true,
            15.0,
        );
    }
    *y += B_SETS.len() as f32 * SET_ROW + 4.0;
    let add = Rect::from_min_size(pos2(left, *y), vec2(width.min(240.0), ROW));
    let _ = w::button(
        ui,
        add,
        "mmn.b.add",
        ctx.pick("テクスチャセットを追加", "Add Texture Set"),
        false,
        true,
        None,
        Some("add"),
    );
    *y += ROW + GAP;
    first
}

/// 開いた選び: 1 行目のマテリアルのチェックの行。
fn popup(ui: &mut Ui, ctx: &mut SketchCtx, anchor: Rect) {
    let entries: Vec<MenuEntry<usize>> = MATERIALS
        .iter()
        .enumerate()
        .map(|(m, name)| {
            let mine = B_SETS[0].materials.contains(&m);
            let mut e = MenuEntry::item(*name, m).checked(mine);
            if let (false, Some(o), MenuEntry::Item { shortcut, .. }) = (mine, b_owner(m), &mut e) {
                *shortcut = Some(o.to_owned());
            }
            e
        })
        .collect();
    let id = Id::new("multimat.new.popup");
    let mut state: Option<PopupState> = ui.data(|d| d.get_temp(id));
    if state.is_none() && ctx.still {
        state = Some(PopupState::new(ui.ctx(), anchor).with_min_width(anchor.width()));
    }
    if let Some(mut open) = state {
        let outcome = menu::show(ui.ctx(), id, &mut open, &entries, &[anchor]);
        if matches!(outcome, PopupOutcome::Open) || ctx.still {
            ui.data_mut(|d| d.insert_temp(id, open));
        } else {
            ui.data_mut(|d| d.remove::<PopupState>(id));
        }
    }
}
