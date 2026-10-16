//! レイヤーの欄の下書き（A・B）が共通で使う、ダミーの行と、行・操作の帯の描き方。部品は `yolu_app::ui::widgets` の物だけ。

use egui::{pos2, vec2, Color32, Painter, Rect, Sense, Ui};
use yolu_app::ui::theme as t;
use yolu_app::ui::widgets::{self as w, Align};

use crate::page::SketchCtx;

pub const ROW_HEIGHT: f32 = 30.0;
pub const TOOLBAR_HEIGHT: f32 = 30.0;
/// 1 段の字下げ。
const INDENT: f32 = 14.0;

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    /// ラスター（サムネイルは色の見本）。
    Raster(Color32),
    /// 塗りつぶし（色）。
    Fill(Color32),
    Adjustment,
    Group,
}

pub struct DummyRow {
    pub name: (&'static str, &'static str),
    pub kind: Kind,
    pub depth: usize,
    pub mask: bool,
    pub locked: bool,
    /// 合成方法の名前（日本語・英語）。
    pub blend: (&'static str, &'static str),
    /// 不透明度（0〜100）。
    pub opacity: f32,
}

pub const ROWS: &[DummyRow] = &[
    DummyRow {
        name: ("レイヤー 3", "Layer 3"),
        kind: Kind::Raster(Color32::from_rgb(222, 150, 80)),
        depth: 0,
        mask: true,
        locked: false,
        blend: ("通常", "Normal"),
        opacity: 100.0,
    },
    DummyRow {
        name: ("グループ 1", "Group 1"),
        kind: Kind::Group,
        depth: 0,
        mask: false,
        locked: false,
        blend: ("通常", "Normal"),
        opacity: 100.0,
    },
    DummyRow {
        name: ("レイヤー 2", "Layer 2"),
        kind: Kind::Raster(Color32::from_rgb(96, 170, 120)),
        depth: 1,
        mask: false,
        locked: false,
        blend: ("乗算", "Multiply"),
        opacity: 80.0,
    },
    DummyRow {
        name: ("塗りつぶし 1", "Fill 1"),
        kind: Kind::Fill(Color32::from_rgb(70, 120, 200)),
        depth: 0,
        mask: false,
        locked: false,
        blend: ("スクリーン", "Screen"),
        opacity: 60.0,
    },
    DummyRow {
        name: ("調整 1", "Adjustment 1"),
        kind: Kind::Adjustment,
        depth: 0,
        mask: false,
        locked: false,
        blend: ("通常", "Normal"),
        opacity: 100.0,
    },
    DummyRow {
        name: ("レイヤー 1", "Layer 1"),
        kind: Kind::Raster(Color32::from_rgb(160, 160, 170)),
        depth: 0,
        mask: false,
        locked: true,
        blend: ("通常", "Normal"),
        opacity: 100.0,
    },
];

/// サムネイル（レイヤーの種類の見た目）。
pub fn thumb(painter: &Painter, r: Rect, kind: Kind) {
    match kind {
        Kind::Raster(c) => {
            w::checker(painter, r, 3.0);
            w::fill(
                painter,
                Rect::from_min_size(r.min, vec2(r.width() * 0.6, r.height())),
                c,
            );
            w::outline(painter, r, t::BORDER, 1.0, 0.0);
        }
        Kind::Fill(c) => {
            w::checker(painter, r, 3.0);
            w::rounded(painter, r, c, 3.0);
            w::outline(painter, r, t::BORDER, 1.0, 3.0);
        }
        Kind::Adjustment => {
            w::rounded(painter, r, t::PANEL_HEADER, 3.0);
            w::icon(painter, r, "tune", t::TEXT_DIM, 15.0);
        }
        Kind::Group => {
            w::rounded(painter, r, t::PANEL_HEADER, 3.0);
            w::icon(painter, r, "folder_open", t::TEXT_DIM, 15.0);
        }
    }
}

/// マスクのサムネイル（白と灰色。`target` なら青い枠）。
pub fn mask_thumb(painter: &Painter, r: Rect, target: bool) {
    w::checker(painter, r, 3.0);
    w::fill(
        painter,
        Rect::from_min_size(r.min, vec2(r.width() * 0.5, r.height())),
        Color32::WHITE,
    );
    w::fill(
        painter,
        Rect::from_min_max(pos2(r.center().x, r.top()), r.max),
        Color32::from_gray(90),
    );
    w::outline(painter, r, t::BORDER, 1.0, 0.0);
    if target {
        w::outline(painter, r.expand(2.0), t::ACCENT, 2.0, 2.0);
    }
}

/// 行 1 つ（目・グループの開閉・サムネイル・マスク・名前・ロック）。名前の右端は、行の右端から `name_right` 内。
/// 押すと選ぶ・目を押すと表示を切り替える。名前の矩形を返す。
pub fn row(
    ui: &mut Ui,
    ctx: &mut SketchCtx,
    list: Rect,
    r: Rect,
    i: usize,
    d: &DummyRow,
    name_right: f32,
) -> Rect {
    let selected = ctx.values.index("layers.selected", 0) == i;
    let visible_key = format!("layers.visible.{i}");
    let visible = ctx.values.bool(&visible_key, true);
    let response = ui.interact(
        r.intersect(list),
        ui.make_persistent_id(("sketch.row", i)),
        Sense::click(),
    );
    if response.clicked() {
        ctx.values.set_index("layers.selected", i);
    }
    let painter = ui.painter_at(list);
    if selected {
        w::fill(&painter, r, t::ACCENT_SOFT);
        w::fill(
            &painter,
            Rect::from_min_size(r.min, vec2(3.0, r.height())),
            t::ACCENT,
        );
    } else if response.hovered() {
        w::fill(&painter, r, t::CONTROL_HOVER);
    }
    w::hline(&painter, r.left(), r.right(), r.bottom() - 1.0, t::BORDER);

    let eye = Rect::from_min_size(
        pos2(r.left() + 4.0, r.top() + 3.0),
        vec2(24.0, r.height() - 6.0),
    );
    let mut x = eye.right() + 4.0 + INDENT * d.depth as f32;
    let group = d.kind == Kind::Group;
    let fold = Rect::from_min_size(pos2(x, r.top() + 3.0), vec2(INDENT, r.height() - 6.0));
    if group {
        x += INDENT;
    }
    let side = r.height() - 8.0;
    let thumb_rect = Rect::from_min_size(pos2(x, r.top() + 4.0), vec2(side, side));
    x = thumb_rect.right() + 4.0;
    let mask_rect = Rect::from_min_size(pos2(x, r.top() + 4.0), vec2(side, side));
    if d.mask {
        x = mask_rect.right() + 4.0;
    }
    let name_rect = Rect::from_min_max(
        pos2(x + 2.0, r.top() + 4.0),
        pos2(r.right() - name_right, r.bottom() - 4.0),
    );

    let lang = ctx.lang;
    let tip = |ja: &'static str, en: &'static str| lang.pick(ja, en);
    let eye_tip = if visible {
        tip("非表示にする", "Hide")
    } else {
        tip("表示する", "Show")
    };
    if w::icon_button(
        ui,
        eye,
        ("sketch.eye", i),
        if visible {
            "visibility"
        } else {
            "visibility_off"
        },
        eye_tip,
        false,
        true,
        16.0,
    )
    .clicked()
    {
        ctx.values.set_bool(&visible_key, !visible);
    }
    if group {
        let _ = w::icon_button(
            ui,
            fold,
            ("sketch.fold", i),
            "expand_more",
            tip("グループを閉じる", "Collapse group"),
            false,
            true,
            14.0,
        );
    }
    let painter = ui.painter_at(list);
    thumb(&painter, thumb_rect, d.kind);
    if selected {
        // 画素が対象のときは、マスクが無くても青い枠
        w::outline(&painter, thumb_rect.expand(2.0), t::ACCENT, 2.0, 2.0);
    }
    if d.mask {
        mask_thumb(&painter, mask_rect, false);
    }
    let name = ctx.pick(d.name.0, d.name.1);
    let color = if !visible {
        t::TEXT_DIM
    } else if selected {
        Color32::WHITE
    } else {
        t::TEXT
    };
    let shown = w::fit(&painter, name, name_rect.width(), t::LABEL);
    w::text(
        &painter,
        name_rect,
        &shown,
        t::LABEL.with_color(color),
        Align::Left,
    );
    if d.locked {
        let mark = Rect::from_min_size(pos2(r.right() - 24.0, r.top()), vec2(20.0, r.height()));
        w::icon(&painter, mark, "lock", t::TEXT_DIM, 13.0);
    }
    name_rect
}

/// 下の操作の帯（左に 8 つ・右に 3 つ。狭いパネルでは間隔を詰める）。
pub fn toolbar(ui: &mut Ui, ctx: &SketchCtx, bar: Rect) {
    w::fill(ui.painter(), bar, t::PANEL_HEADER);
    let left: [(&str, &str, &str); 8] = [
        ("add", "新規レイヤー", "New layer"),
        ("format_color_fill", "塗りつぶしレイヤー", "Fill layer"),
        ("tune", "調整レイヤー", "Adjustment layer"),
        ("auto_awesome", "フィルターを追加", "Add filter"),
        ("texture", "ジェネレーターを追加", "Add generator"),
        ("folder", "グループ化", "Group"),
        ("vignette", "マスクを追加", "Add mask"),
        ("keyboard_arrow_down", "クリッピング", "Clipping"),
    ];
    let right: [(&str, &str, &str); 3] = [
        ("expand_less", "上へ", "Move up"),
        ("expand_more", "下へ", "Move down"),
        ("delete", "削除", "Delete"),
    ];
    let pitch = ((bar.width() - 8.0) / 11.0).min(27.0);
    let button = |x: f32| Rect::from_min_size(pos2(x, bar.top() + 3.0), vec2(pitch - 1.0, 24.0));
    let size = |base: f32| base.min((pitch - 2.0).max(10.0));
    for (k, (icon, ja, en)) in left.iter().enumerate() {
        let _ = w::icon_button(
            ui,
            button(bar.left() + 4.0 + pitch * k as f32),
            ("sketch.toolbar.left", k),
            icon,
            ctx.pick(ja, en),
            false,
            true,
            size(17.0),
        );
    }
    let x = bar.right() - 4.0 - pitch * 3.0;
    for (k, (icon, ja, en)) in right.iter().enumerate() {
        let _ = w::icon_button(
            ui,
            button(x + pitch * k as f32),
            ("sketch.toolbar.right", k),
            icon,
            ctx.pick(ja, en),
            false,
            true,
            size(17.0),
        );
    }
}
