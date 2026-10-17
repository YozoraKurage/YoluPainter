//! アイコン（`ui::icons` が持つ全部の名前を、`widgets::icon` で描く）。

use egui::{pos2, vec2, Rect, Ui};
use yolu_app::ui::theme as t;
use yolu_app::ui::widgets as w;

use super::{Entry, Section};
use crate::page::SketchCtx;

/// `yolu-app` の `ui/icons.rs` が読むアイコンの名前（同じ並び。`names_match_the_icons_the_app_loads` が食い違いを落とす）。
pub const NAMES: &[&str] = &[
    "add",
    "delete",
    "expand_less",
    "expand_more",
    "chevron_right",
    "arrow_drop_down",
    "check",
    "restart_alt",
    "visibility",
    "visibility_off",
    "swap_horiz",
    "color_square",
    "target",
    "stylus",
    "opacity",
    "paint_brush",
    "shapes",
    "square",
    "layers",
    "library",
    "palette",
    "tune",
    "view_in_ar",
    "flip",
    "snap_ruler",
    "snap_special",
    "rotate_90_degrees_cw",
    "rotate_90_degrees_ccw",
    "warning",
    "warning_circle",
    "info",
    "close",
    "lock",
    "link_off",
    "sync",
    "texture",
    "folder_open",
    "folder",
    "format_color_fill",
    "vignette",
    "content_copy",
    "keyboard_arrow_down",
    "invert_colors",
    "contrast",
    "blur_on",
    "3d_rotation",
    "light_mode",
    "data_scatter",
    "ink_stroke",
    "grid_dots",
    "uv_wireframe",
    "local_fire_department",
    "arrow_maximize",
    "arrow_minimize",
    "copy_add",
    "save",
    "search",
    "import",
    "accessibility",
    "anchor",
    "auto_awesome",
    "clip_below",
    "generator",
    "tools/brush",
    "tools/brush_selected",
    "tools/eraser",
    "tools/eraser_selected",
    "tools/fill",
    "tools/fill_selected",
    "tools/gradient",
    "tools/gradient_selected",
    "tools/polygon-fill",
    "tools/polygon-fill_selected",
    "tools/eyedropper",
    "tools/eyedropper_selected",
    "tools/id-select",
    "tools/id-select_selected",
    "select_all",
    "deselect",
    "tools/select-rectangle",
    "tools/select-rectangle_selected",
    "tools/select-ellipse",
    "tools/select-ellipse_selected",
    "tools/lasso",
    "tools/lasso_selected",
    "tools/select-polygon",
    "tools/select-polygon_selected",
    "tools/magic-wand",
    "tools/magic-wand_selected",
    "tools/select-pen",
    "tools/select-pen_selected",
    "shape_union",
    "shape_subtract",
    "shape_intersect",
    "edit",
    "quick_mask",
    "more_horizontal",
    "tools/move",
    "tools/move_selected",
    "tools/liquify",
    "tools/liquify_selected",
    "lock_filled",
    "lock_transparency",
    "arrow_move",
    "flip_vertical",
    "tools/path",
    "tools/path_selected",
    "tools/ruler",
    "tools/ruler_selected",
    "conversion_path",
    "window_minimize",
    "window_maximize",
    "window_restore",
    "error_circle",
    "document_copy",
    "record",
    "stop",
    "play",
    "video_clip",
];

/// 1 つのアイコンの枠（アイコンと、その下の名前）。
const CELL: egui::Vec2 = egui::vec2(88.0, 58.0);

/// 幅（点）に並ぶ列の数。
fn columns(width: f32) -> usize {
    ((width - 12.0) / CELL.x).floor().max(1.0) as usize
}

/// 幅に合わせた見本の高さ。
fn height_at(width: f32) -> f32 {
    NAMES.len().div_ceil(columns(width)) as f32 * CELL.y + 12.0
}

pub fn section() -> Section {
    Section {
        title: ("アイコン", "Icons"),
        entries: vec![Entry::new(
            "widgets::icon",
            ("アプリが持つアイコンの全部", "Every icon the app has"),
            height_at(640.0),
            grid,
        )
        .wide()
        .height_at(height_at)],
    }
}

fn grid(ui: &mut Ui, sample: Rect, _: &mut SketchCtx) {
    let p = ui.painter().clone();
    let columns = columns(sample.width());
    for (i, name) in NAMES.iter().enumerate() {
        let cell = Rect::from_min_size(
            pos2(
                sample.left() + 6.0 + (i % columns) as f32 * CELL.x,
                sample.top() + 6.0 + (i / columns) as f32 * CELL.y,
            ),
            CELL,
        );
        w::icon(
            &p,
            Rect::from_min_size(cell.min + vec2(0.0, 2.0), vec2(cell.width(), 24.0)),
            name,
            t::TEXT,
            20.0,
        );
        // 長い名前は 2 行に折り返す（真ん中に寄せる）
        let galley = p.layout(
            (*name).to_owned(),
            t::LABEL_SMALL.font(),
            t::LABEL_SMALL.color,
            cell.width() - 4.0,
        );
        let at = pos2(cell.center().x - galley.size().x / 2.0, cell.top() + 28.0);
        p.galley(at, galley, t::LABEL_SMALL.color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_height_grows_as_the_width_shrinks() {
        assert!(height_at(300.0) > height_at(600.0));
        assert!(height_at(600.0) >= height_at(1200.0));
        assert!(height_at(10.0).is_finite());
    }

    #[test]
    fn names_match_the_icons_the_app_loads() {
        let source = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../yolu-app/src/ui/icons.rs"
        ))
        .expect("yolu-app の ui/icons.rs が読めない");
        let start = source.find("icon_files!(").expect("icon_files! が無い");
        let body = &source[start..];
        let end = body.find("\n);").expect("icon_files! の終わりが無い");
        let loaded: Vec<&str> = body[..end]
            .lines()
            .filter_map(|l| l.trim().strip_prefix('"')?.strip_suffix("\","))
            .collect();
        assert_eq!(loaded, NAMES);
    }
}
