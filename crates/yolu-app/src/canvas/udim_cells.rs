//! 2D のキャンバスの UDIM のシートの重ね描き（セルの境の線・タイルの番号・空のセルの暗さ）。

/// 2D のキャンバスに、UDIM のシートのセルの境の線と、各セルの左上の隅にタイルの番号を重ねる。空のセルは暗くする（番号は出さない）。
pub fn paint(
    painter: &egui::Painter,
    view: &crate::canvas::view::CanvasView,
    doc: &yolu_core::Document,
) {
    use crate::ui::theme as t;
    use egui::{pos2, Color32, Shape, Stroke};
    let Some(layout) = doc.udim() else {
        return;
    };
    let line = Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 110));
    let shade = Color32::from_rgba_unmultiplied(0, 0, 0, 150);
    let font = egui::FontId::proportional(11.0);
    for cell in 0..layout.cell_count() {
        let r = layout.cell_rect(cell);
        let (x0, y0) = (f64::from(r.x), f64::from(r.y));
        let (x1, y1) = (f64::from(r.x + r.width), f64::from(r.y + r.height));
        let corners = vec![
            view.to_screen(x0, y0),
            view.to_screen(x1, y0),
            view.to_screen(x1, y1),
            view.to_screen(x0, y1),
        ];
        let Some(tile) = layout.tile_of_cell(cell) else {
            painter.add(Shape::convex_polygon(corners, shade, line));
            continue;
        };
        painter.add(Shape::closed_line(corners, line));
        // 番号はセルの左上の隅（文書は左下が原点なので、上の辺は y1）。画面に対して横書きのまま、隅から少し内へ
        let at = view.to_screen(x0, y1);
        let label = tile.to_string();
        let galley = painter.layout_no_wrap(label, font.clone(), t::TEXT);
        let size = galley.size();
        let pad = egui::vec2(4.0, 2.0);
        let rect = egui::Rect::from_min_size(pos2(at.x + 3.0, at.y + 3.0), size + pad * 2.0);
        painter.rect_filled(rect, 2.0, Color32::from_rgba_unmultiplied(0, 0, 0, 140));
        painter.galley(rect.min + pad, galley, t::TEXT);
    }
}
