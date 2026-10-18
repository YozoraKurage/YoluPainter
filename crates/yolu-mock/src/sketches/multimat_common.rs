//! 1 つのテクスチャセットに複数のマテリアルを結び付ける案の下書きが共通で使う、ダミーの値と「プロジェクト設定」のウィンドウの描き方。
//! 寸法と並びは本物のウィンドウ（`newproject/window.rs`）に合わせ、部品は `yolu_app::ui` の物だけ。

use egui::{pos2, vec2, Id, Painter, Rect, Ui, Vec2};
use yolu_app::ui::menu::{self, Entry as MenuEntry, PopupOutcome, PopupState};
use yolu_app::ui::theme as t;
use yolu_app::ui::widgets::{self as w, Align};
use yolu_app::ui::window::{self, Frame, Spec};

use crate::page::SketchCtx;

const WIDTH: f32 = 560.0;
const PAD: f32 = 16.0;
const ROW: f32 = 24.0;
const GAP: f32 = 8.0;
const LABEL_W: f32 = 168.0;
const FOOT: f32 = 52.0;
const SET_ROW: f32 = 26.0;
/// マテリアルの箱の幅（本物の行と同じ）。
const MATERIAL_W: f32 = 156.0;
const SIZE_W: f32 = 88.0;

/// モデルのマテリアル（モデルの並び順）。
pub const MATERIALS: [&str; 7] = ["Face", "Ear", "Eye", "Tongue", "Body", "Cloth", "Hair"];

/// ダミーのセット 1 行（`materials` は MATERIALS の添え字）。
pub struct DummySet {
    pub name: &'static str,
    pub materials: &'static [usize],
}

/// 下書きのセット: Ear のセットは、Ear のマテリアルを Face のセットへ移して空になった行。
pub const SETS: [DummySet; 4] = [
    DummySet {
        name: "Face",
        materials: &[0, 1],
    },
    DummySet {
        name: "Ear",
        materials: &[],
    },
    DummySet {
        name: "Eye",
        materials: &[2],
    },
    DummySet {
        name: "Body",
        materials: &[4, 5, 6, 3],
    },
];

/// マテリアル `m` が属するセットの名前。
pub fn owner(m: usize) -> Option<&'static str> {
    SETS.iter()
        .find(|s| s.materials.contains(&m))
        .map(|s| s.name)
}

/// ウィンドウの見せ方。
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// 1 行目のマテリアルの箱を開いた形（空になった Ear の行は、案によらない中立の形のまま）。
    Dropdown,
    /// 案 A: 空になった行は残り、マテリアルの欄に警告。
    KeepWarn,
}

/// 閉じたときのマテリアルの箱の文字（並べて入りきらなければ「先頭 ほか n」）。
pub fn material_label(
    p: &Painter,
    lang: yolu_app::lang::Lang,
    names: &[&str],
    width: f32,
) -> String {
    let full = names.join(", ");
    if names.len() <= 1 || w::text_width(p, &full, t::LABEL) <= width {
        return full;
    }
    let n = names.len() - 1;
    match lang {
        yolu_app::lang::Lang::Ja => format!("{} ほか {n}", names[0]),
        yolu_app::lang::Lang::En => format!("{} and {n} more", names[0]),
    }
}

fn height(summary: bool, rows: usize) -> f32 {
    let mut h = window::HEADER_HEIGHT + PAD;
    h += ROW + GAP; // メッシュ
    if summary {
        h += ROW + GAP;
    }
    h += 18.0 + rows as f32 * SET_ROW + 4.0 + ROW + GAP; // セットの一覧と追加
    h += ROW + GAP; // 補間方法
    h += ROW + GAP; // 法線の形式
    h + 4.0 + FOOT
}

/// 「プロジェクト設定」のウィンドウを画面の真ん中に描く。
pub fn config_window(ui: &mut Ui, ctx: &mut SketchCtx, mode: Mode) {
    let summary = mode == Mode::KeepWarn;
    let spec = Spec {
        title: ctx.pick("プロジェクト設定", "Project Configuration"),
        icon: Some("tune"),
        size: vec2(WIDTH, height(summary, SETS.len())),
        modal: false,
        close_label: ctx.pick("ウィンドウを閉じる", "Close Window"),
    };
    let id = Id::new("multimat.window");
    let mut offset = Vec2::ZERO;
    let mut anchor: Option<Rect> = None;
    window::show(ui.ctx(), id, &spec, &mut offset, false, |ui, frame| {
        anchor = body(ui, frame, ctx, mode);
    });
    if mode == Mode::Dropdown {
        if let Some(anchor) = anchor {
            popup(ui, ctx, anchor);
        }
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

/// 本体を描く。最初の行のマテリアルの箱の矩形を返す。
fn body(ui: &mut Ui, frame: &Frame, ctx: &mut SketchCtx, mode: Mode) -> Option<Rect> {
    let lang = ctx.lang;
    let body = frame.body;
    let p = ui.painter().clone();
    let left = body.left() + PAD;
    let width = body.width() - 2.0 * PAD;
    let mut y = body.top() + PAD;
    let control = |y: f32| Rect::from_min_max(pos2(left + LABEL_W, y), pos2(left + width, y + ROW));

    // メッシュ
    label(&p, left, y, ctx.pick("メッシュ", "Mesh"));
    {
        let c = control(y);
        let folder = Rect::from_min_size(pos2(c.right() - 26.0, y), vec2(26.0, ROW));
        let _ = w::icon_button(
            ui,
            folder,
            "mm.choose",
            "folder_open",
            ctx.pick("モデルを選ぶ", "Choose a Model"),
            false,
            false,
            17.0,
        );
        let box_rect = Rect::from_min_max(c.min, pos2(c.right() - 30.0, y + ROW));
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

    // 状態の行（案 A: マテリアルの無いセットがある）
    if mode == Mode::KeepWarn {
        let text = ctx.pick("マテリアルなしのセット 1", "1 set without a material");
        let tw = w::text_width(&p, text, t::LABEL_DIM);
        let r = Rect::from_min_size(pos2(left + LABEL_W, y), vec2(tw + 28.0, ROW));
        w::icon(
            &p,
            Rect::from_min_size(r.min, vec2(18.0, ROW)),
            "link_off",
            t::WARNING,
            14.0,
        );
        w::text(
            &p,
            Rect::from_min_max(pos2(r.left() + 20.0, y), r.max),
            text,
            t::LABEL_DIM.with_color(t::WARNING),
            Align::Left,
        );
        y += ROW + GAP;
    }

    // テクスチャセット
    w::text(
        &p,
        Rect::from_min_size(pos2(left, y), vec2(width, 16.0)),
        ctx.pick("テクスチャセット", "Texture Sets"),
        t::HEADER.with_color(t::TEXT_DIM),
        Align::Left,
    );
    y += 18.0;
    let mut first_material = None;
    for (i, set) in SETS.iter().enumerate() {
        let row = Rect::from_min_size(
            pos2(left, y + i as f32 * SET_ROW),
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
        let empty = set.materials.is_empty();

        // 名前
        let _ = w::text_field(ui, name, ("mm.name", i), set.name, None, false);

        // 大きさ
        let _ = w::dropdown(
            ui,
            size,
            ("mm.size", i),
            None,
            "2048 × 2048",
            None,
            true,
            0.0,
        );

        // マテリアル
        let names: Vec<&str> = set.materials.iter().map(|&m| MATERIALS[m]).collect();
        let shown = material_label(&p, lang, &names, material.width() - 26.0);
        let (_, b) = w::dropdown(
            ui,
            material,
            ("mm.material", i),
            None,
            &shown,
            None,
            true,
            0.0,
        );
        if empty {
            if mode == Mode::KeepWarn {
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
            } else {
                w::text(
                    &p,
                    Rect::from_min_max(
                        pos2(b.left() + 7.0, b.top()),
                        pos2(b.right() - 22.0, b.bottom()),
                    ),
                    "—",
                    t::LABEL.with_color(t::TEXT_DIM),
                    Align::Left,
                );
            }
        }
        if i == 0 {
            first_material = Some(b);
        }

        // 消す
        let _ = w::icon_button(
            ui,
            remove,
            ("mm.remove", i),
            "delete",
            ctx.pick("このテクスチャセットを消す", "Remove this texture set"),
            false,
            true,
            15.0,
        );
    }
    y += SETS.len() as f32 * SET_ROW + 4.0;
    let add = Rect::from_min_size(pos2(left, y), vec2(width.min(240.0), ROW));
    let _ = w::button(
        ui,
        add,
        "mm.add",
        ctx.pick("テクスチャセットを追加", "Add Texture Set"),
        false,
        true,
        None,
        Some("add"),
    );
    y += ROW + GAP;

    // 補間方法（大きさを変えないので押せない）
    label(&p, left, y, ctx.pick("補間方法", "Resampling"));
    let _ = w::dropdown(
        ui,
        control(y),
        "mm.resampling",
        None,
        ctx.pick("自動", "Automatic"),
        None,
        false,
        0.0,
    );
    y += ROW + GAP;

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
        "mm.normal",
        None,
        ctx.pick("OpenGL（Y+、Unity）", "OpenGL (Y+, Unity)"),
        None,
        true,
        0.0,
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
        "mm.cancel",
        ctx.pick("キャンセル", "Cancel"),
        false,
        true,
        None,
        None,
    );
    let _ = w::button(
        ui,
        ok,
        "mm.ok",
        ctx.pick("適用", "Apply"),
        true,
        true,
        None,
        None,
    );

    first_material
}

/// マテリアルの箱を開いた形（マテリアルごとのチェックの行。ほかのセットに属している行は、右にそのセットの名前を薄く）。
fn popup(ui: &mut Ui, ctx: &mut SketchCtx, anchor: Rect) {
    let set = &SETS[0];
    let entries: Vec<MenuEntry<usize>> = MATERIALS
        .iter()
        .enumerate()
        .map(|(m, name)| {
            let mut e = MenuEntry::item(*name, m).checked(set.materials.contains(&m));
            if !set.materials.contains(&m) {
                if let (MenuEntry::Item { shortcut, .. }, Some(o)) = (&mut e, owner(m)) {
                    *shortcut = Some(o.to_owned());
                }
            }
            e
        })
        .collect();
    let id = Id::new("multimat.popup");
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
