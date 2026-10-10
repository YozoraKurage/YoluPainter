//! アイコン（Unity 版の Editor/UI/Icons から写した白の 48 px の PNG。Fluent UI System Icons と Phosphor、どちらも MIT。
//! 出どころは assets/icons/THIRD-PARTY-NOTICES.md）。egui-wgpu は mipmap を作らないので、読むときに 16・24・32 px に縮めた版も
//! 作り、描く大きさ（物理の画素）に近いものを使う（48 px を 15 px に直に縮めると線がちらつく）。

use std::collections::HashMap;
use std::sync::Arc;

use egui::{Color32, ColorImage, Painter, Rect, TextureHandle, TextureOptions};

macro_rules! icon_files {
    ($($name:literal),* $(,)?) => {
        &[$(($name, include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icons/", $name, ".png")) as &[u8])),*]
    };
}

const FILES: &[(&str, &[u8])] = icon_files!(
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
);

const SIZES: [u32; 4] = [16, 24, 32, 48];

pub struct Icons {
    map: HashMap<&'static str, Vec<(u32, TextureHandle)>>,
}

impl Icons {
    pub fn load(ctx: &egui::Context) -> Icons {
        let mut map = HashMap::new();
        for (name, bytes) in FILES {
            let Ok(decoded) = image::load_from_memory_with_format(bytes, image::ImageFormat::Png)
            else {
                continue;
            };
            let mut rgba = decoded.to_rgba8();
            // 縮めるときに透明の画素の色がにじまないよう、乗算済みにしてから縮める
            for p in rgba.pixels_mut() {
                let a = p[3] as u32;
                for c in 0..3 {
                    p[c] = ((p[c] as u32 * a + 127) / 255) as u8;
                }
            }
            let mut set = Vec::new();
            for size in SIZES {
                let scaled = if rgba.width() == size {
                    rgba.clone()
                } else {
                    image::imageops::resize(
                        &rgba,
                        size,
                        size,
                        image::imageops::FilterType::Triangle,
                    )
                };
                let image = ColorImage::from_rgba_premultiplied(
                    [size as usize, size as usize],
                    scaled.as_raw(),
                );
                let handle =
                    ctx.load_texture(format!("icon:{name}@{size}"), image, TextureOptions::LINEAR);
                set.push((size, handle));
            }
            map.insert(*name, set);
        }
        // 同梱の MIT アイコンをツールにも共用する。
        for (alias, source) in [
            ("tools/shape", "shapes"),
            ("tools/shape_selected", "shapes"),
        ] {
            if let Some(set) = map.get(source).cloned() {
                map.insert(alias, set);
            }
        }
        Icons { map }
    }

    pub fn has(&self, name: &str) -> bool {
        self.map.contains_key(name)
    }

    /// rect の中央に size（論理の点）の大きさで、色を付けて描く。無いアイコンは名前の頭文字。
    pub fn paint(&self, painter: &Painter, rect: Rect, name: &str, color: Color32, size: f32) {
        let at = Rect::from_center_size(rect.center(), egui::vec2(size, size));
        match self.map.get(name) {
            Some(set) => {
                let physical = size * painter.ctx().pixels_per_point();
                let texture = set
                    .iter()
                    .find(|(s, _)| *s as f32 >= physical - 0.5)
                    .unwrap_or_else(|| set.last().expect("icon sizes"))
                    .1
                    .id();
                painter.image(
                    texture,
                    at,
                    Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    color,
                );
            }
            None => {
                let letter: String = name
                    .chars()
                    .next()
                    .map(|c| c.to_uppercase().collect())
                    .unwrap_or_else(|| "?".into());
                painter.text(
                    at.center(),
                    egui::Align2::CENTER_CENTER,
                    letter,
                    egui::FontId::proportional(size * 0.7),
                    color,
                );
            }
        }
    }
}

/// 文脈にアイコンを置く（部品は `get` で取り出す。毎回の引数で渡さないため）。
pub fn install(ctx: &egui::Context) {
    let icons = Arc::new(Icons::load(ctx));
    ctx.data_mut(|d| d.insert_temp(egui::Id::new("yolu.icons"), icons));
}

pub fn get(ctx: &egui::Context) -> Option<Arc<Icons>> {
    ctx.data(|d| d.get_temp::<Arc<Icons>>(egui::Id::new("yolu.icons")))
}

/// アイコンを描く（入っていなければ頭文字）。
pub fn paint(painter: &Painter, rect: Rect, name: &str, color: Color32, size: f32) {
    match get(painter.ctx()) {
        Some(icons) => icons.paint(painter, rect, name, color, size),
        None => {
            let letter: String = name
                .chars()
                .next()
                .map(|c| c.to_uppercase().collect())
                .unwrap_or_else(|| "?".into());
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                letter,
                egui::FontId::proportional(size * 0.7),
                color,
            );
        }
    }
}
