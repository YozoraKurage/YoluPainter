//! 配布用に保存（案 A）: 今のウィンドウの区分を残し、アセットの種類（画像・スマート素材・ブラシ）を開くと、その種類のアセットが 1 行ずつ出る
//! （チェック・サムネイル・名前・大きさ。使っていない物は「未使用」の印、使っているのに外した物は警告の印）。種類の行のチェックは、中の全部を
//! 入れる・外すの一括（一部だけのときは横線）。アセット以外の除ける物は、今のウィンドウと同じ 1 行ずつの切り替え。

use egui::{pos2, vec2, Rect, Ui};
use yolu_app::ui::theme as t;
use yolu_app::ui::widgets::{self as w, Align};

use super::distribute_common::{self as common, Kind, ASSETS};
use crate::page::SketchCtx;

const PREFIX: &str = "distribute-a";
const GROUP_HEIGHT: f32 = 30.0;
const ASSET_HEIGHT: f32 = 32.0;
const OTHER_HEIGHT: f32 = 30.0;
const SIDE: f32 = 16.0;

const GROUPS: [Kind; 3] = [Kind::Image, Kind::Smart, Kind::Brush];

#[derive(Clone, Copy, PartialEq)]
enum Tri {
    None,
    Some,
    All,
}

pub fn draw(ui: &mut Ui, ctx: &mut SketchCtx) {
    common::show_window(ui, ctx, vec2(480.0, 570.0), |ui, ctx, content| {
        let p = ui.painter().clone();
        let left = content.left() + 14.0;
        let mut y = content.top() + 8.0;

        for (g, kind) in GROUPS.iter().enumerate() {
            let members: Vec<usize> = (0..ASSETS.len())
                .filter(|i| ASSETS[*i].kind == *kind)
                .collect();
            let on = members
                .iter()
                .filter(|i| common::included(ctx, PREFIX, **i))
                .count();
            let state = if on == 0 {
                Tri::None
            } else if on == members.len() {
                Tri::All
            } else {
                Tri::Some
            };
            let open_key = format!("{PREFIX}.open.{g}");
            let open = ctx.values.bool(&open_key, g != 1);
            let row =
                Rect::from_min_size(pos2(left, y), vec2(content.width() - 28.0, GROUP_HEIGHT));
            // 開閉の矢印
            let chevron = Rect::from_min_size(row.min, vec2(22.0, row.height()));
            if w::icon_button(
                ui,
                chevron,
                ("distribute-a.fold", g),
                if open { "expand_more" } else { "chevron_right" },
                if open {
                    ctx.pick("閉じる", "Collapse")
                } else {
                    ctx.pick("開く", "Expand")
                },
                false,
                true,
                16.0,
            )
            .clicked()
            {
                ctx.values.set_bool(&open_key, !open);
            }
            let boxr = Rect::from_min_size(
                pos2(row.left() + 28.0, row.center().y - SIDE / 2.0),
                vec2(SIDE, SIDE),
            );
            if tri_box(
                ui,
                boxr,
                ("distribute-a.group", g),
                state,
                ctx.pick("含める", "Include"),
            ) {
                let next = state != Tri::All;
                for i in &members {
                    common::set_included(ctx, PREFIX, *i, next);
                }
            }
            let label = kind.label(ctx);
            let label_rect = Rect::from_min_max(pos2(boxr.right() + 8.0, row.top()), row.max);
            w::text(&p, label_rect, label, t::LABEL_BOLD, Align::Left);
            let total: f32 = members
                .iter()
                .filter(|i| common::included(ctx, PREFIX, **i))
                .map(|i| ASSETS[*i].mb)
                .sum();
            w::text(
                &p,
                Rect::from_min_max(row.min, pos2(row.right() - 4.0, row.bottom())),
                &common::size_text(total),
                t::LABEL_DIM,
                Align::Right,
            );
            y += GROUP_HEIGHT;

            if open {
                for i in members {
                    let r = Rect::from_min_size(pos2(left, y), vec2(row.width(), ASSET_HEIGHT));
                    asset_row(ui, ctx, r, i, 26.0);
                    y += ASSET_HEIGHT;
                }
            }
            y += 2.0;
        }

        y += 4.0;
        w::hline(&p, left, content.right() - 14.0, y, t::SEPARATOR);
        y += 6.0;
        for (i, asset) in ASSETS.iter().enumerate() {
            if matches!(asset.kind, Kind::Other(_)) {
                let r =
                    Rect::from_min_size(pos2(left, y), vec2(content.width() - 28.0, OTHER_HEIGHT));
                asset_row(ui, ctx, r, i, 0.0);
                y += OTHER_HEIGHT;
            }
        }
    });
}

/// アセット 1 行（チェックの箱は `indent` だけ右から始まる）。
fn asset_row(ui: &mut Ui, ctx: &mut SketchCtx, r: Rect, i: usize, indent: f32) {
    let on = common::included(ctx, PREFIX, i);
    let asset = &ASSETS[i];
    let boxr = Rect::from_min_size(
        pos2(r.left() + 28.0 + indent, r.center().y - SIDE / 2.0),
        vec2(SIDE + 2.0, SIDE),
    );
    let next = w::toggle(
        ui,
        boxr,
        ("distribute-a.asset", i),
        "",
        on,
        Some(ctx.pick("含める", "Include")),
        true,
    );
    if next != on {
        common::set_included(ctx, PREFIX, i, next);
    }
    let p = ui.painter().clone();
    common::asset_content(&p, ctx, r, boxr.right() + 10.0, 4.0, asset, on);
}

/// 3 つの状態のチェックの箱（全部・一部・なし）。押されたら true。
fn tri_box(ui: &mut Ui, r: Rect, id: impl egui::AsIdSalt, state: Tri, tip: &str) -> bool {
    let id = ui.make_persistent_id(id);
    let response = ui.interact(r.expand(3.0), id, egui::Sense::click());
    let p = ui.painter().clone();
    if state == Tri::None {
        w::rounded(&p, r, t::CONTROL_BG, 3.0);
        w::outline(
            &p,
            r,
            if response.hovered() {
                t::ACCENT_DIM
            } else {
                t::SEPARATOR
            },
            1.0,
            3.0,
        );
    } else {
        w::rounded(&p, r, t::ACCENT, 3.0);
        if state == Tri::All {
            w::icon(&p, r, "check", egui::Color32::WHITE, 14.0);
        } else {
            w::fill(
                &p,
                Rect::from_center_size(r.center(), vec2(r.width() - 7.0, 2.0)),
                egui::Color32::WHITE,
            );
        }
    }
    let _ = response.clone().on_hover_text(tip);
    response.clicked()
}
