//! 焼いたメッシュマップを 2D のキャンバスに重ねて見る（読むだけ。何も描かない）。
//!
//! 見るのは今のセットのマップ 1 枚か、テクセルの由来（`Coverage`: 緑 = 覆う、赤 = UV の重なり、青 = 余白）。表示用の 8 bit
//! （`BakedMeshMap::to_rgba8`。覆わないテクセルは透明）を、文書の表示と同じ頁（`PAGE` 画素）に分けたテクスチャにして、文書の上に
//! 同じ写し（拡大・パン・回転・反転）で描く。正本（16 bit）は変えず、保存もしない。マップの大きさが文書と違っても、UV の
//! 上で文書に合わせる（古いマップを見ても場所は合う）。

use egui::{pos2, Color32, ColorImage, Painter, TextureHandle, TextureOptions};
use yolu_core::mesh_maps::MeshMapKind;

use crate::canvas::display::{CanvasDisplay, NEAREST_FROM_PIXEL_SIZE, PAGE};
use crate::canvas::view::CanvasView;
use crate::state::AppState;

/// キャンバスに重ねて見るもの。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MeshMapView {
    #[default]
    None,
    Coverage,
    Kind(MeshMapKind),
}

struct Page {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    texture: TextureHandle,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Key {
    set: u32,
    view: MeshMapView,
    revision: u64,
    nearest: bool,
}

/// 重ね表示のテクスチャ（見るものが変わったときだけ作り直す）。
#[derive(Default)]
pub struct Overlay {
    key: Option<Key>,
    /// マップの大きさ。
    size: (u32, u32),
    pages: Vec<Page>,
}

impl Overlay {
    /// 頁の数（試験用）。
    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    fn rebuild(&mut self, ctx: &egui::Context, key: Key, rgba: &[u8], (width, height): (u32, u32)) {
        let options = TextureOptions {
            magnification: if key.nearest {
                egui::TextureFilter::Nearest
            } else {
                egui::TextureFilter::Linear
            },
            minification: egui::TextureFilter::Linear,
            wrap_mode: egui::TextureWrapMode::ClampToEdge,
            mipmap_mode: None,
        };
        self.pages.clear();
        let mut y = 0;
        while y < height {
            let mut x = 0;
            while x < width {
                let (pw, ph) = (PAGE.min(width - x), PAGE.min(height - y));
                let mut pixels = Vec::with_capacity((pw * ph) as usize);
                for row in 0..ph {
                    let start = (((y + row) * width + x) * 4) as usize;
                    for p in rgba[start..start + (pw * 4) as usize].as_chunks::<4>().0 {
                        pixels.push(Color32::from_rgba_unmultiplied(p[0], p[1], p[2], p[3]));
                    }
                }
                let texture = ctx.load_texture(
                    format!("meshmap-overlay-{x}-{y}"),
                    ColorImage::new([pw as usize, ph as usize], pixels),
                    options,
                );
                self.pages.push(Page {
                    x,
                    y,
                    width: pw,
                    height: ph,
                    texture,
                });
                x += PAGE;
            }
            y += PAGE;
        }
        self.size = (width, height);
        self.key = Some(key);
    }
}

/// 文書の上に重ねて描く（`display.paint` の後。見るものが無ければ何もしない）。
pub fn paint(painter: &Painter, app: &mut AppState, view: &CanvasView) {
    app.sync_mesh_map_view();
    let which = app.bake.view;
    if which == MeshMapView::None {
        if app.bake.overlay.key.is_some() {
            app.bake.overlay.key = None;
            app.bake.overlay.pages.clear();
        }
        return;
    }
    let set = app.sets.current();
    let map = match which {
        MeshMapView::None => return,
        MeshMapView::Coverage => set.mesh_maps.iter().next(),
        MeshMapView::Kind(kind) => set.mesh_maps.get(kind),
    }
    .cloned();
    let Some(map) = map else {
        return;
    };
    let nearest = view.pixel_size() >= NEAREST_FROM_PIXEL_SIZE;
    let key = Key {
        set: set.uid,
        view: which,
        revision: set.mesh_maps.revision(),
        nearest,
    };
    if app.bake.overlay.key != Some(key) {
        let rgba = map.to_rgba8(which == MeshMapView::Coverage);
        app.bake.overlay.rebuild(
            painter.ctx(),
            key,
            &rgba,
            (map.width() as u32, map.height() as u32),
        );
    }
    // 文書の矩形（画素）に、マップを UV の上で合わせる
    let (doc_w, doc_h) = (app.doc.width() as f64, app.doc.height() as f64);
    let overlay = &app.bake.overlay;
    let (mw, mh) = (overlay.size.0 as f64, overlay.size.1 as f64);
    for page in &overlay.pages {
        let x0 = page.x as f64 / mw * doc_w;
        let y0 = page.y as f64 / mh * doc_h;
        let x1 = (page.x + page.width) as f64 / mw * doc_w;
        let y1 = (page.y + page.height) as f64 / mh * doc_h;
        CanvasDisplay::quad(
            painter,
            page.texture.id(),
            view,
            [(x0, y0), (x1, y0), (x1, y1), (x0, y1)],
            [
                pos2(0.0, 0.0),
                pos2(1.0, 0.0),
                pos2(1.0, 1.0),
                pos2(0.0, 1.0),
            ],
        );
    }
}

/// 重ね表示の名前（見ていなければ None）。
pub fn view_name(app: &AppState) -> Option<String> {
    let lang = app.lang;
    match app.bake.view {
        MeshMapView::None => None,
        MeshMapView::Coverage => Some(lang.pick("UV の範囲", "UV Coverage").to_owned()),
        MeshMapView::Kind(kind) => Some(super::kind_label(lang, kind).to_owned()),
    }
}
