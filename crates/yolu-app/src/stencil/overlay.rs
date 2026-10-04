//! ステンシルの重ね表示（2D のキャンバスと 3D のビューの上）と、プロパティの欄の見本の絵。重ね表示は画面に貼り付いた半透明の画像で、
//! 量として読むなら輝度（反転）の灰色に、色として読むなら画像の色にして、塗られる値の見た目にする。繰り返すときは表示域いっぱいに広げる
//! （片側に多くても 64 枚）。T を押している・動かしているあいだは、画像 1 枚の枠と中心の十字を白と黒の二重の線で出す。

use egui::epaint::Vertex;
use egui::{
    pos2, Color32, ColorImage, Context, Mesh, Painter, Pos2, Rect, Shape, Stroke, TextureFilter,
    TextureHandle, TextureId, TextureOptions, TextureWrapMode,
};
use yolu_core::brush::luminance;
use yolu_core::{StencilMode, StencilTiling};

use super::{Preview, StencilFrame, StencilState};

/// 重ね表示の絵（作った鍵と一緒に覚える）。鍵が変わったときだけ作り直す。
pub struct OverlayTexture {
    key: (u64, StencilMode, bool, StencilTiling),
    handle: TextureHandle,
}

/// 見本の絵。
pub struct ThumbTexture {
    id: u64,
    handle: TextureHandle,
}

fn color_image(preview: &Preview, mode: Option<(StencilMode, bool)>) -> ColorImage {
    let mut rgba = preview.rgba.clone();
    if let Some((StencilMode::Mask, invert)) = mode {
        for p in rgba.as_chunks_mut::<4>().0 {
            let mut v = luminance(p[0], p[1], p[2]);
            if invert {
                v = 255 - v;
            }
            p[0] = v;
            p[1] = v;
            p[2] = v;
        }
    }
    ColorImage::from_rgba_unmultiplied(preview.size, &rgba)
}

fn repeats(tiling: StencilTiling) -> (bool, bool) {
    (
        matches!(tiling, StencilTiling::Horizontal | StencilTiling::Both),
        matches!(tiling, StencilTiling::Vertical | StencilTiling::Both),
    )
}

impl StencilState {
    /// 重ね表示の絵（画像・読み方・反転・繰り返しが変わったときだけ作り直す）。
    fn overlay_texture(&mut self, ctx: &Context) -> Option<TextureId> {
        let image = self.image.as_ref()?;
        let mode = yolu_core::BrushStencil::resolve(self.mode, &image.image);
        let key = (image.id, mode, self.invert, self.tiling);
        if let Some(t) = self.overlay.as_ref().filter(|t| t.key == key) {
            return Some(t.handle.id());
        }
        let (rx, ry) = repeats(self.tiling);
        let options = TextureOptions {
            magnification: TextureFilter::Linear,
            minification: TextureFilter::Linear,
            wrap_mode: if rx || ry {
                TextureWrapMode::Repeat
            } else {
                TextureWrapMode::ClampToEdge
            },
            mipmap_mode: None,
        };
        let handle = ctx.load_texture(
            "stencil-overlay",
            color_image(image.preview(), Some((mode, self.invert))),
            options,
        );
        let id = handle.id();
        self.overlay = Some(OverlayTexture { key, handle });
        Some(id)
    }

    /// プロパティの欄の見本の絵（元の画像の見た目）。
    pub fn thumb_texture(&mut self, ctx: &Context) -> Option<TextureId> {
        let image = self.image.as_ref()?;
        if let Some(t) = self.thumb.as_ref().filter(|t| t.id == image.id) {
            return Some(t.handle.id());
        }
        let handle = ctx.load_texture(
            "stencil-thumb",
            color_image(image.thumb(), None),
            TextureOptions::LINEAR,
        );
        let id = handle.id();
        self.thumb = Some(ThumbTexture {
            id: image.id,
            handle,
        });
        Some(id)
    }

    /// 重ね表示の絵を作ったか（試験用）。
    pub fn has_overlay_texture(&self) -> bool {
        self.overlay.is_some()
    }
}

/// 画像 1 枚の枠と中心の十字（白と黒の二重の線）。
fn draw_frame(painter: &Painter, f: &StencilFrame) {
    let corners = f.corners();
    let c = pos2(f.center.0 as f32, f.center.1 as f32);
    let cross = [
        [pos2(c.x - 7.0, c.y), pos2(c.x + 7.0, c.y)],
        [pos2(c.x, c.y - 7.0), pos2(c.x, c.y + 7.0)],
    ];
    for (width, color) in [
        (3.0, Color32::from_black_alpha(140)),
        (1.2, Color32::from_white_alpha(230)),
    ] {
        let stroke = Stroke::new(width, color);
        painter.add(Shape::closed_line(corners.to_vec(), stroke));
        for line in cross {
            painter.add(Shape::line_segment(line, stroke));
        }
    }
}

/// 表示域 view（画面の点。painter は view で切ったもの）にステンシルを重ねる。重ねるのは、画像があり、N を押していないとき
/// （T を押しているあいだは常に）。
pub fn draw(painter: &Painter, st: &mut StencilState, view: Rect) {
    if !st.shown() {
        return;
    }
    let Some(frame) = st.frame_in(view) else {
        return;
    };
    let Some(texture) = st.overlay_texture(painter.ctx()) else {
        return;
    };
    let (rx, ry) = repeats(st.tiling);
    // 繰り返す軸は、表示域の四隅が画像のどこに来るか（画像の幅・高さを 1 として）から範囲を広げる
    let (mut u0, mut u1, mut v0, mut v1) = (0.0f64, 1.0f64, 0.0f64, 1.0f64);
    if rx || ry {
        let (mut min_u, mut max_u, mut min_v, mut max_v) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
        for corner in [
            view.left_top(),
            view.right_top(),
            view.left_bottom(),
            view.right_bottom(),
        ] {
            let (x, y) = frame.to_image(corner.x as f64, corner.y as f64);
            let (u, v) = (x / frame.image_width as f64, y / frame.image_height as f64);
            min_u = min_u.min(u);
            max_u = max_u.max(u);
            min_v = min_v.min(v);
            max_v = max_v.max(v);
        }
        if rx {
            u0 = min_u.floor().max(-64.0);
            u1 = max_u.ceil().min(65.0);
        }
        if ry {
            v0 = min_v.floor().max(-64.0);
            v1 = max_v.ceil().min(65.0);
        }
    }
    if u1 > u0 && v1 > v0 {
        // 回していない枠の中の、画像の画素 (u, v)（v は上向き）の位置（中心からのずれ。y は下向き）
        let (w, h) = (frame.width, frame.height);
        let (left, right) = ((u0 - 0.5) * w, (u1 - 0.5) * w);
        let (top, bottom) = ((0.5 - v1) * h, (0.5 - v0) * h);
        let corners: [Pos2; 4] = [
            frame.rotated(left, top),
            frame.rotated(right, top),
            frame.rotated(right, bottom),
            frame.rotated(left, bottom),
        ];
        // テクスチャの行 0 は画像の上（v = 1）
        let uvs = [
            pos2(u0 as f32, (1.0 - v1) as f32),
            pos2(u1 as f32, (1.0 - v1) as f32),
            pos2(u1 as f32, (1.0 - v0) as f32),
            pos2(u0 as f32, (1.0 - v0) as f32),
        ];
        let tint = Color32::WHITE.gamma_multiply(st.opacity);
        let mut mesh = Mesh::with_texture(texture);
        for (pos, uv) in corners.iter().zip(uvs) {
            mesh.vertices.push(Vertex {
                pos: *pos,
                uv,
                color: tint,
            });
        }
        mesh.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);
        painter.add(Shape::mesh(mesh));
    }
    if st.handling() {
        draw_frame(painter, &frame);
    }
}
