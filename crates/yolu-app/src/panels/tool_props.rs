//! 左のドックの「ツールプロパティ」と「ブラシサイズ」のパネル（クリスタのツールプロパティ・ブラシサイズのパレットに当たる）。どちらも今のツールに合わせて
//! 中身が替わり、サブツールの一覧（`subtools`）とは別のタブなので、並びの中で別々に動かせる・別ウィンドウへ出せる。
//!
//! - ツールプロパティ: 今のツールの設定の全部（ツールの表 `tools` の `properties`。値はオプションバーと同じ状態）。描くツールと選択のツール（表の `paint_channels`）には、
//!   終わりに「塗るチャンネル」の区分（複数のチャンネルを一度に塗る設定。`material`）が付く。マスクに描くあいだは、その区分がレイヤーマスクの欄に替わる。
//!   入りきらなければ縦にスクロールする。
//! - ブラシサイズ: 大きさを持つツールの決まった大きさの丸。持たないツールでは空。
//!
//! 画面には名前と値だけを出し、説明はツールチップ。

use egui::{pos2, vec2, Rect, Ui};

use super::brushes;
use crate::state::AppState;
use crate::ui::scroll::Scroll;
use crate::ui::widgets::Rows;

/// ツールプロパティのパネル。
pub fn show(ui: &mut Ui, app: &mut AppState) {
    let r = ui.max_rect();
    ui.advance_cursor_after_rect(r);
    let ctx = ui.ctx().clone();
    app.brushes.samples.begin_frame(ctx.cumulative_pass_nr());
    let tool = app.tool;
    let def = tool.def();
    let index = tool as usize;
    let bar = Scroll::begin(
        ui,
        r,
        app.subtools.ui.props_content[index],
        &mut app.subtools.ui.props_scroll[index],
    );
    let scroll = app.subtools.ui.props_scroll[index];
    let area = Rect::from_min_max(
        pos2(r.left(), r.top() - scroll),
        pos2(r.right() - bar.reserved(), r.bottom()),
    );
    let outer = ui.clip_rect();
    ui.set_clip_rect(r.intersect(outer));
    let mut rows = Rows::compact(area, 0.0);
    (def.properties)(ui, app, &mut rows, &ctx);
    if def.paint_channels {
        rows.indent = 0.0;
        if app.m2.edit_mask {
            super::layer_props::mask_tab(ui, app, &mut rows);
        } else {
            super::material::paint_channels_section(ui, app, &mut rows);
        }
    }
    rows.indent = 0.0;
    rows.space(6.0);
    // 中身の高さは次のフレームの組み立てに使う（変わったら、すぐ組み立て直す）
    if (rows.used() - app.subtools.ui.props_content[index]).abs() > 0.5 {
        ctx.request_repaint();
    }
    app.subtools.ui.props_content[index] = rows.used();
    ui.set_clip_rect(outer);
    if def.paint_channels && app.m2.edit_mask {
        // マスクの欄のスライダーのドラッグを離したら、まとめていた変更を 1 回の Undo にする
        super::properties::end_drag_when_released(ui, app);
    }
    bar.end(
        ui,
        "subtools.props.scroll",
        &mut app.subtools.ui.props_scroll[index],
    );
}

/// ブラシサイズのパネル（大きさを持つツールだけ丸を出す。持たないツールは空）。
pub fn show_sizes(ui: &mut Ui, app: &mut AppState) {
    let r = ui.max_rect();
    ui.advance_cursor_after_rect(r);
    if !app.tool.def().sized {
        return;
    }
    // 丸の段の数は幅で決まる。パネルに収まるなら全幅の並び、収まらなければつまみの分を引いた幅の並び
    let wide = brushes::sizes_height(r.width());
    let content = if wide <= r.height() {
        wide
    } else {
        brushes::sizes_height(r.width() - crate::ui::scroll::BAR_WIDTH)
    };
    let bar = Scroll::begin(ui, r, content, &mut app.subtools.ui.sizes_scroll);
    let scroll = app.subtools.ui.sizes_scroll;
    let area = Rect::from_min_max(
        pos2(r.left(), r.top() - scroll),
        pos2(r.right() - bar.reserved(), r.bottom()),
    );
    let outer = ui.clip_rect();
    ui.set_clip_rect(r.intersect(outer));
    brushes::sizes_body(
        ui,
        app,
        Rect::from_min_size(area.min, vec2(area.width(), content)),
    );
    ui.set_clip_rect(outer);
    bar.end(
        ui,
        "subtools.sizes.scroll",
        &mut app.subtools.ui.sizes_scroll,
    );
}
