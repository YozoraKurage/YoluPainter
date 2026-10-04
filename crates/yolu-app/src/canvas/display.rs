//! 文書の合成を見せるテクスチャ。文書を PAGE 画素の頁に分け（大きい文書でも GPU のテクスチャの上限に収める）、
//! 初めと大きさが変わったときだけ全部を作り、あとは core が「変わった」と言うタイルだけを合成して、そのタイルの範囲だけを
//! テクスチャに上げる（egui の `set_partial`。egui-wgpu がその範囲だけ `write_texture` する）。
//! テクスチャの行は core と同じ下から上のまま（行を並べ替えない）。上下は描くときの UV で返す。
//! 見せるだけの写しで、保存の正本ではない（正本は core の straight RGBA8）。乗算済みへの変換は表示のためだけ。

use egui::{
    epaint::Vertex, pos2, Color32, ColorImage, Mesh, Painter, Pos2, Rect, Shape, TextureHandle,
    TextureOptions,
};

use super::view::CanvasView;
use crate::engine::{Channel, Document, Rect as DocRect, RowOrder};
use crate::ui::theme as t;

/// 頁の一辺（タイルの倍数）。
pub const PAGE: u32 = 2048;
/// 市松の 1 マス（画面の点）。
const CHECKER_CELL: f32 = 8.0;

/// 画素 1 つがこの点数以上に拡大されたら、補間せずに画素の角を見せる（それ未満は滑らかに）。
pub const NEAREST_FROM_PIXEL_SIZE: f32 = 2.0;

fn options(nearest: bool) -> TextureOptions {
    TextureOptions {
        magnification: if nearest {
            egui::TextureFilter::Nearest
        } else {
            egui::TextureFilter::Linear
        },
        minification: egui::TextureFilter::Linear,
        wrap_mode: egui::TextureWrapMode::ClampToEdge,
        mipmap_mode: None,
    }
}

struct Page {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    texture: TextureHandle,
}

/// 上げた量の記録（試験と状態の表示用）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UploadStats {
    /// 最後の `sync` で上げたタイルの数（作り直しなら全部）。
    pub last_tiles: usize,
    /// 最後の `sync` が全部を作り直したか。
    pub last_rebuilt: bool,
    /// これまでに上げたタイルの数。
    pub total_tiles: usize,
}

#[derive(Default)]
pub struct CanvasDisplay {
    /// 最後に読んだ文書（テクスチャセットを替える・開き直すと別の文書になる。通し番号は文書ごとなので、替わったら全部を作り直す）。
    doc_id: u128,
    /// 最後に読んだチャンネル（替わったら全部を作り直す）。
    channel: Option<Channel>,
    /// 最後に読んだ core の変化の通し番号（次はこれより後の変化だけを読む）。
    serial: u64,
    /// 合成の受け皿（毎回の確保を避ける）。
    buffer: Vec<u8>,
    size: (u32, u32),
    pages: Vec<Page>,
    checker: Option<TextureHandle>,
    /// 今のテクスチャが画素の角を見せる（Nearest）か。
    nearest: bool,
    /// 次の `sync` で、この補間で作り直す。
    want_nearest: bool,
    pub stats: UploadStats,
}

fn to_image(width: u32, height: u32, straight: &[u8]) -> ColorImage {
    let pixels = straight
        .as_chunks::<4>()
        .0
        .iter()
        .map(|p| Color32::from_rgba_unmultiplied(p[0], p[1], p[2], p[3]))
        .collect();
    ColorImage::new([width as usize, height as usize], pixels)
}

impl CanvasDisplay {
    pub fn new() -> CanvasDisplay {
        CanvasDisplay::default()
    }

    /// 文書の変わった所をテクスチャに上げる。上げたタイルの数を返す。
    pub fn sync(&mut self, ctx: &egui::Context, doc: &Document) -> usize {
        self.sync_channel(ctx, doc, Channel::Color)
    }

    /// 文書の変わった所をテクスチャに上げる（`channel` の合成を出す）。上げたタイルの数を返す。
    pub fn sync_channel(&mut self, ctx: &egui::Context, doc: &Document, channel: Channel) -> usize {
        let (w, h) = (doc.width(), doc.height());
        let changed = doc.changed_tiles(channel, self.serial);
        let serial = doc.change_serial();
        if self.size != (w, h)
            || self.pages.is_empty()
            || self.nearest != self.want_nearest
            || self.doc_id != doc.id()
            || self.channel != Some(channel)
            || changed.is_none()
        {
            self.nearest = self.want_nearest;
            self.doc_id = doc.id();
            self.channel = Some(channel);
            self.serial = serial;
            self.pages.clear();
            let mut y = 0;
            while y < h {
                let mut x = 0;
                while x < w {
                    let (pw, ph) = (PAGE.min(w - x), PAGE.min(h - y));
                    let rect = DocRect::new(x, y, pw, ph);
                    self.buffer.resize((pw * ph * 4) as usize, 0);
                    let image = match doc.composite_into(
                        channel,
                        rect,
                        &mut self.buffer,
                        RowOrder::BottomUp,
                    ) {
                        Ok(()) => to_image(pw, ph, &self.buffer),
                        Err(_) => {
                            ColorImage::filled([pw as usize, ph as usize], Color32::TRANSPARENT)
                        }
                    };
                    let texture = ctx.load_texture(
                        format!("canvas-page-{x}-{y}"),
                        image,
                        options(self.nearest),
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
            self.size = (w, h);
            let ts = doc.tile_size();
            let tiles = (w.div_ceil(ts) * h.div_ceil(ts)) as usize;
            self.stats = UploadStats {
                last_tiles: tiles,
                last_rebuilt: true,
                total_tiles: self.stats.total_tiles + tiles,
            };
            return tiles;
        }
        let changed = changed.unwrap_or_default();
        self.serial = serial;
        for coord in &changed {
            let Some(region) = doc.tile_rect(*coord) else {
                continue;
            };
            if region.width == 0 || region.height == 0 {
                continue;
            }
            self.buffer
                .resize((region.width * region.height * 4) as usize, 0);
            if doc
                .composite_into(channel, region, &mut self.buffer, RowOrder::BottomUp)
                .is_err()
            {
                continue;
            }
            let pixels = &self.buffer;
            if let Some(page) = self.pages.iter_mut().find(|p| {
                region.x >= p.x
                    && region.x < p.x + p.width
                    && region.y >= p.y
                    && region.y < p.y + p.height
            }) {
                page.texture.set_partial(
                    [(region.x - page.x) as usize, (region.y - page.y) as usize],
                    to_image(region.width, region.height, pixels),
                    options(self.nearest),
                );
            }
        }
        self.stats = UploadStats {
            last_tiles: changed.len(),
            last_rebuilt: false,
            total_tiles: self.stats.total_tiles + changed.len(),
        };
        changed.len()
    }

    fn checker_texture(&mut self, ctx: &egui::Context) -> egui::TextureId {
        self.checker
            .get_or_insert_with(|| {
                let image = ColorImage::new(
                    [2, 2],
                    vec![
                        t::CHECKER_LIGHT,
                        t::CHECKER_DARK,
                        t::CHECKER_DARK,
                        t::CHECKER_LIGHT,
                    ],
                );
                ctx.load_texture(
                    "canvas-checker",
                    image,
                    TextureOptions {
                        magnification: egui::TextureFilter::Nearest,
                        minification: egui::TextureFilter::Nearest,
                        wrap_mode: egui::TextureWrapMode::Repeat,
                        mipmap_mode: None,
                    },
                )
            })
            .id()
    }

    /// 文書の矩形（画素の座標 x0..x1 × y0..y1）を、写しで回した 4 隅の四角として描く。
    fn quad(
        painter: &Painter,
        texture: egui::TextureId,
        view: &CanvasView,
        corners: [(f64, f64); 4],
        uvs: [Pos2; 4],
    ) {
        let mut mesh = Mesh::with_texture(texture);
        for (c, uv) in corners.iter().zip(uvs) {
            mesh.vertices.push(Vertex {
                pos: view.to_screen(c.0, c.1),
                uv,
                color: Color32::WHITE,
            });
        }
        mesh.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);
        painter.add(Shape::mesh(mesh));
    }

    /// 透明の市松と合成の絵を描く（painter はキャンバスの矩形で切ったもの）。
    pub fn paint(&mut self, painter: &Painter, view: &CanvasView) {
        // 補間の切り替えは境を越えたときだけ（作り直しは次のフレームの `sync`）
        let want = view.pixel_size() >= NEAREST_FROM_PIXEL_SIZE;
        if want != self.want_nearest {
            self.want_nearest = want;
            painter.ctx().request_repaint();
        }
        let (w, h) = (self.size.0 as f64, self.size.1 as f64);
        if self.pages.is_empty() {
            return;
        }
        // 市松は画面の大きさが一定（拡大しても 8 点のマス）で、画像と一緒に回る
        let checker = self.checker_texture(painter.ctx());
        let k = view.pixel_size() / (2.0 * CHECKER_CELL);
        let (cu, cv) = (w as f32 * k, h as f32 * k);
        let corners = [(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)];
        Self::quad(
            painter,
            checker,
            view,
            corners,
            [pos2(0.0, cv), pos2(cu, cv), pos2(cu, 0.0), pos2(0.0, 0.0)],
        );
        for page in &self.pages {
            let (x0, y0) = (page.x as f64, page.y as f64);
            let (x1, y1) = (x0 + page.width as f64, y0 + page.height as f64);
            // テクスチャの行 0 が文書の y0（下）なので、UV の v はそのまま y に比例する
            Self::quad(
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

    /// 画素の角を見せる補間か（試験用）。
    pub fn is_nearest(&self) -> bool {
        self.nearest
    }

    /// 頁の数（試験用）。
    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    /// 画面のこの点の下の、テクスチャに載っている画素の矩形（試験用。無ければ None）。
    pub fn page_rect(&self, index: usize) -> Option<Rect> {
        self.pages.get(index).map(|p| {
            Rect::from_min_size(
                pos2(p.x as f32, p.y as f32),
                egui::vec2(p.width as f32, p.height as f32),
            )
        })
    }
}
