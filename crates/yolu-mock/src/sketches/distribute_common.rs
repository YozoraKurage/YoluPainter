//! 配布用に保存の下書き（A・B）が共通で使う、ダミーのアセットと、行の中身・ウィンドウの枠の描き方。部品は `yolu_app::ui` の物だけ。
//! 向きは「写しに含める」（チェックが入っている物が入る）。今のウィンドウは「除く」にチェックを入れる向きなので、問いにする。

use egui::{pos2, vec2, Color32, Painter, Rect, Shape, Stroke, Ui, Vec2};
use yolu_app::ui::theme as t;
use yolu_app::ui::widgets::{self as w, Align};
use yolu_app::ui::window::{self, Spec};

use crate::gallery::{remember, store};
use crate::page::SketchCtx;

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    Image,
    Smart,
    Brush,
    /// アセット以外の、除ける物（今のウィンドウの種類）。アイコンの名前を持つ。
    Other(&'static str),
}

impl Kind {
    pub fn label(self, ctx: &SketchCtx) -> &'static str {
        match self {
            Kind::Image => ctx.pick("画像", "Images"),
            Kind::Smart => ctx.pick("スマート素材", "Smart Assets"),
            Kind::Brush => ctx.pick("ブラシ", "Brushes"),
            Kind::Other(_) => ctx.pick("ほか", "Other"),
        }
    }
}

pub struct Asset {
    pub kind: Kind,
    pub name: (&'static str, &'static str),
    /// 大きさ（MB）。
    pub mb: f32,
    /// 今の作品の中で使っているか（使っていない物は印）。
    pub used: bool,
    /// 写しに含めるか（使っていない物は外す・使っている物は含めるのが既定。1 つだけ、使っているのに外した物を入れてある）。
    pub included: bool,
    /// サムネイルの色。
    pub color: Color32,
}

const fn asset(
    kind: Kind,
    name: (&'static str, &'static str),
    mb: f32,
    used: bool,
    included: bool,
    color: Color32,
) -> Asset {
    Asset {
        kind,
        name,
        mb,
        used,
        included,
        color,
    }
}

const GRAY: Color32 = Color32::from_rgb(120, 120, 128);

pub const ASSETS: &[Asset] = &[
    asset(
        Kind::Image,
        ("肌", "Skin"),
        4.8,
        true,
        true,
        Color32::from_rgb(226, 178, 150),
    ),
    asset(
        Kind::Image,
        ("服", "Cloth"),
        2.1,
        true,
        false,
        Color32::from_rgb(90, 130, 196),
    ),
    asset(
        Kind::Image,
        ("髪", "Hair"),
        1.4,
        true,
        true,
        Color32::from_rgb(120, 84, 66),
    ),
    asset(
        Kind::Image,
        ("影のマスク", "Shadow Mask"),
        0.9,
        false,
        false,
        GRAY,
    ),
    asset(
        Kind::Smart,
        ("金属", "Metal"),
        3.6,
        true,
        true,
        Color32::from_rgb(168, 176, 188),
    ),
    asset(
        Kind::Smart,
        ("布", "Fabric"),
        2.2,
        false,
        false,
        Color32::from_rgb(170, 120, 150),
    ),
    asset(Kind::Brush, ("ペン 1", "Pen 1"), 0.3, true, true, GRAY),
    asset(Kind::Brush, ("水彩", "Watercolor"), 0.6, true, true, GRAY),
    asset(Kind::Brush, ("鉛筆", "Pencil"), 0.4, false, false, GRAY),
    asset(
        Kind::Other("import"),
        ("PSD の原本", "Original PSDs"),
        24.6,
        true,
        false,
        GRAY,
    ),
    asset(
        Kind::Other("folder_open"),
        ("素材の出どころのパス", "Asset source paths"),
        0.1,
        true,
        false,
        GRAY,
    ),
    asset(
        Kind::Other("view_in_ar"),
        ("モデルの参照", "Model reference"),
        0.1,
        true,
        false,
        GRAY,
    ),
    asset(
        Kind::Other("uv_wireframe"),
        ("メッシュマップ", "Mesh maps"),
        12.0,
        true,
        true,
        GRAY,
    ),
    asset(
        Kind::Other("sync"),
        ("Unity のマテリアルの値", "Unity material values"),
        0.2,
        true,
        false,
        GRAY,
    ),
];

pub fn size_text(mb: f32) -> String {
    format!("{mb:.1} MB")
}

pub fn included(ctx: &SketchCtx, prefix: &str, i: usize) -> bool {
    ctx.values
        .bool(&format!("{prefix}.in.{i}"), ASSETS[i].included)
}

pub fn set_included(ctx: &mut SketchCtx, prefix: &str, i: usize, on: bool) {
    ctx.values.set_bool(&format!("{prefix}.in.{i}"), on);
}

/// サムネイル（種類ごとの見た目。素材が無いので色の見本）。
pub fn thumb(p: &Painter, r: Rect, a: &Asset) {
    match a.kind {
        Kind::Image => {
            w::checker(p, r, 4.0);
            w::fill(
                p,
                Rect::from_min_size(r.min, vec2(r.width() * 0.68, r.height())),
                a.color,
            );
            w::outline(p, r, t::BORDER, 1.0, 0.0);
        }
        Kind::Smart => {
            w::rounded(p, r, a.color, 3.0);
            w::icon(
                p,
                r,
                "texture",
                Color32::from_white_alpha(170),
                r.height() - 8.0,
            );
            w::outline(p, r, t::BORDER, 1.0, 3.0);
        }
        Kind::Brush => {
            w::rounded(p, r, t::CONTROL_BG, 3.0);
            let points: Vec<egui::Pos2> = (0..=10)
                .map(|k| {
                    let u = k as f32 / 10.0;
                    pos2(
                        r.left() + 4.0 + u * (r.width() - 8.0),
                        r.center().y + (u * std::f32::consts::TAU).sin() * r.height() * 0.22,
                    )
                })
                .collect();
            p.add(Shape::line(points, Stroke::new(2.0, t::TEXT_DIM)));
            w::outline(p, r, t::BORDER, 1.0, 3.0);
        }
        Kind::Other(name) => {
            w::rounded(p, r, t::PANEL_HEADER, 3.0);
            w::icon(p, r, name, t::TEXT_DIM, r.height() - 10.0);
        }
    }
}

/// 「未使用」の小さな印（名前と同じ行の右側に置く。幅を返す）。
fn unused_chip(p: &Painter, right: f32, row: Rect, label: &str) -> f32 {
    let width = w::text_width(p, label, t::LABEL_SMALL) + 12.0;
    let r = Rect::from_min_size(pos2(right - width, row.center().y - 9.0), vec2(width, 18.0));
    w::rounded(p, r, t::CONTROL_BG, 9.0);
    w::outline(p, r, t::SEPARATOR, 1.0, 9.0);
    w::text(
        p,
        r,
        label,
        t::LABEL_SMALL.with_color(t::TEXT_DIM),
        Align::Center,
    );
    width
}

/// アセット 1 行の中身（サムネイル・名前・未使用の印・使っているのに外した印・大きさ）。`x0` は左の始まり。`dim` は外した行（名前を暗く）。
/// 右の端は `row.right() - pad`。名前の矩形は返さない（呼ぶ側は触らない）。
pub fn asset_content(
    p: &Painter,
    ctx: &SketchCtx,
    row: Rect,
    x0: f32,
    pad: f32,
    a: &Asset,
    included: bool,
) {
    let side = (row.height() - 8.0).min(26.0);
    let thumb_rect = Rect::from_min_size(pos2(x0, row.center().y - side / 2.0), vec2(side, side));
    thumb(p, thumb_rect, a);
    let mut right = row.right() - pad;
    {
        let size = size_text(a.mb);
        let width = 58.0;
        w::text(
            p,
            Rect::from_min_size(pos2(right - width, row.top()), vec2(width, row.height())),
            &size,
            t::LABEL_DIM,
            Align::Right,
        );
        right -= width + 8.0;
    }
    if !a.used {
        right -= unused_chip(p, right, row, ctx.pick("未使用", "Unused")) + 6.0;
    } else if !included && !matches!(a.kind, Kind::Other(_)) {
        // 使っているアセットを外した（見た目が欠ける）印
        let mark = Rect::from_min_size(pos2(right - 18.0, row.top()), vec2(18.0, row.height()));
        w::icon(p, mark, "warning", t::WARNING, 15.0);
        right -= 24.0;
    }
    let name_rect = Rect::from_min_max(
        pos2(thumb_rect.right() + 8.0, row.top()),
        pos2(right, row.bottom()),
    );
    let color = if included { t::TEXT } else { t::TEXT_DIM };
    let name = ctx.pick(a.name.0, a.name.1);
    let shown = w::fit(p, name, name_rect.width(), t::LABEL);
    w::text(
        p,
        name_rect,
        &shown,
        t::LABEL.with_color(color),
        Align::Left,
    );
}

/// ウィンドウの枠（見出し・下の帯の「やめる」「保存…」）を、見本の矩形の真ん中に描く。`content` に、見出しの下から下の帯の上までの矩形を渡す。
pub fn show_window(
    ui: &mut Ui,
    ctx: &mut SketchCtx,
    size: Vec2,
    content: impl FnOnce(&mut Ui, &mut SketchCtx, Rect),
) {
    let area = ui.max_rect();
    ui.advance_cursor_after_rect(area);
    let id = ui.id().with("distribute.window");
    let screen = ui.ctx().content_rect();
    let mut offset: Vec2 = if ctx.still {
        area.center() - screen.center()
    } else {
        remember(ui, "distribute.offset", || area.center() - screen.center())
    };
    let lang = ctx.lang;
    let spec = Spec {
        title: lang.pick("配布用に保存", "Save for Distribution"),
        icon: Some("save"),
        size,
        modal: false,
        close_label: lang.pick("ウィンドウを閉じる", "Close Window"),
    };
    let _ = window::show(ui.ctx(), id, &spec, &mut offset, false, |ui, frame| {
        let body = frame.body;
        let footer_height = 48.0;
        content(
            ui,
            ctx,
            Rect::from_min_max(body.min, pos2(body.right(), body.bottom() - footer_height)),
        );
        let p = ui.painter().clone();
        let footer = Rect::from_min_max(pos2(body.left(), body.bottom() - footer_height), body.max);
        w::fill(&p, footer, t::PANEL_HEADER);
        w::hline(&p, footer.left(), footer.right(), footer.top(), t::BORDER);
        let mut x = footer.right() - 14.0;
        let buttons = [
            (lang.pick("やめる", "Cancel"), false),
            (lang.pick("保存…", "Save…"), true),
        ];
        for (i, (label, primary)) in buttons.into_iter().enumerate().rev() {
            let bw = w::text_width(&p, label, t::LABEL) + 32.0;
            let r = Rect::from_min_size(pos2(x - bw, footer.top() + 10.0), vec2(bw, 28.0));
            x = r.left() - 8.0;
            let _ = w::button(
                ui,
                r,
                id.with(("button", i)),
                label,
                primary,
                true,
                None,
                None,
            );
        }
    });
    if !ctx.still {
        store(ui, "distribute.offset", offset);
    }
}
