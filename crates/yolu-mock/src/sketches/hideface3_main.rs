//! 面を隠す（5 回目）: ハイドのモード（モードのドロップダウンから入る。Esc で前のモードへ戻る）。3D ビューの右から「ハイド」の引き出しが出て
//! （ビューを狭めない）、ボーンは丸（編集のモードの物の印と同じ見た目）で出て押して選ぶ。UV アイランドは、丸の掴める範囲の外で押して選ぶ
//! （Shift で足す）。H で隠し、Alt+H ですべて表示する。ポーズのモードは同じ丸に、選んだボーンの回転のギズモを重ねる。シェルフは左下に固定。

use egui::Ui;

use super::hideface3_common::{draw, Mode, Opts, Pane, ShelfLook};
use super::hideface_common as common;
use crate::page::SketchCtx;

/// ハイドのモードの標準の形: 面を 2 つ選び（Shift で足した形）、もう 1 つにポインターが乗り、ボーン Chest を 1 つ選んでいる。
/// 隠している物は、UV アイランド 1 つと、ボーンで隠した Hand.L。
fn hide_pane(drawer_open: bool) -> Pane {
    Pane {
        key: "main",
        mode: Mode::Hide,
        hidden: vec![common::CUBE_FRONT],
        bone_hidden: true,
        selected_faces: vec![common::TALL_FRONT, common::TALL_SIDE],
        pointed: Some(common::WIDE_FRONT_R),
        selected_bones: vec![2],
        applied: true,
        shelf: ShelfLook::Closed,
        hide_preset: 1,
        pose_preset: 0,
        drawer_open,
        zoom: 1.0,
        shading_icon: true,
    }
}

fn show(ui: &mut Ui, ctx: &mut SketchCtx, pane: Pane) {
    draw(
        ui,
        ctx,
        Opts {
            pane,
            mode_menu: false,
        },
    );
}

/// ハイドのモード、引き出しが開いた形、ボーンの丸を 1 つ選んだ形。
pub fn draw_main(ui: &mut Ui, ctx: &mut SketchCtx) {
    show(ui, ctx, hide_pane(true));
}

/// 引き出しを閉じた形（つまみだけ）。
pub fn draw_drawer_closed(ui: &mut Ui, ctx: &mut SketchCtx) {
    show(ui, ctx, hide_pane(false));
}

/// ズームアウト（丸が小さくなる。下限の 3 点が近い）。
pub fn draw_zoom_out(ui: &mut Ui, ctx: &mut SketchCtx) {
    show(
        ui,
        ctx,
        Pane {
            zoom: 0.5,
            ..hide_pane(true)
        },
    );
}

/// ズームイン（丸が大きくなる。上限の 9 点）。
pub fn draw_zoom_in(ui: &mut Ui, ctx: &mut SketchCtx) {
    show(
        ui,
        ctx,
        Pane {
            zoom: 1.4,
            ..hide_pane(true)
        },
    );
}

/// ポーズのモード: 丸と、選んだボーン（UpperArm.L）の回転のギズモ。
pub fn draw_pose(ui: &mut Ui, ctx: &mut SketchCtx) {
    show(
        ui,
        ctx,
        Pane {
            mode: Mode::Pose,
            selected_faces: vec![],
            pointed: None,
            selected_bones: vec![4],
            ..hide_pane(false)
        },
    );
}

/// シェルフを開いて、ハイドのプリセットの一覧を出した形（持ち手なし。左下に固定）。
pub fn draw_shelf_list(ui: &mut Ui, ctx: &mut SketchCtx) {
    show(
        ui,
        ctx,
        Pane {
            shelf: ShelfLook::OpenWithList,
            selected_faces: vec![],
            pointed: None,
            selected_bones: vec![],
            ..hide_pane(false)
        },
    );
}
