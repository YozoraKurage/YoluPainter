//! 選択範囲の量（0〜255）を、キャンバスの上に色つきの半透明の重ねとして見せる（クイックマスクの赤・選択ペンの途中の被覆）。
//!
//! 量のあるタイルごとに、表示に近い大きさの絵（量 × 色の濃さ）を 1 枚の絵にして、文書と同じ写し（拡大・パン・回転・反転）で描く。
//! 見えているタイルだけを作る。1 面が同じ量のタイルは絵を作らず、色だけの四角で描く。選択範囲が変わったら見えているタイルを
//! 中身の写しで見比べ、変わった絵だけを作り直す（呼ぶ側が変わったタイルを知っていれば、そのタイルだけ）。

use std::collections::{HashMap, HashSet};
use std::hash::Hasher;

use egui::epaint::{Vertex, WHITE_UV};
use egui::{pos2, Color32, ColorImage, Mesh, Painter, Shape, TextureHandle, TextureOptions};

use crate::canvas::display::CanvasDisplay;
use crate::canvas::view::CanvasView;
use crate::engine::{SelectionMask, TileCoord};

/// 絵を持つタイルの数の上限（これを超えて見えているぶんは描かない）。
const MAX_TEXTURES: usize = 1500;

/// 重ねる色と、量が満量のときの濃さ（0〜1）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tint {
    pub rgb: [u8; 3],
    pub alpha: f32,
}

enum Slot {
    /// 1 面が同じ量（0 でない）。
    Solid(u8),
    Texture {
        hash: u64,
        handle: TextureHandle,
    },
}

/// 重ね表示のタイルの絵（選択範囲か色が変わったときだけ作り直す）。
#[derive(Default)]
pub struct TileOverlay {
    slots: HashMap<TileCoord, Slot>,
    last: Option<SelectionMask>,
    tint: Option<Tint>,
    /// 試験が見る: 絵を作った・作り直した数の累計。
    uploads: u64,
}

impl TileOverlay {
    /// 絵を持っているタイルの数（試験用）。
    pub fn texture_count(&self) -> usize {
        self.slots
            .values()
            .filter(|s| matches!(s, Slot::Texture { .. }))
            .count()
    }

    /// 絵を作った・作り直した回数の累計（試験用）。
    pub fn uploads(&self) -> u64 {
        self.uploads
    }

    /// 全部捨てる。
    pub fn clear(&mut self) {
        self.slots.clear();
        self.last = None;
        self.tint = None;
    }

    /// 文書の上に重ねて描く。`changed` は呼ぶ側が知っている、前のフレームから量が変わったタイル（知らなければ None で、見えている
    /// タイルを全部見比べる）。
    pub fn paint(
        &mut self,
        painter: &Painter,
        view: &CanvasView,
        mask: &SelectionMask,
        tint: Tint,
        changed: Option<&[TileCoord]>,
        name: &str,
    ) {
        let ts = mask.tile_size();
        let (cols, rows) = (
            mask.width().div_ceil(ts) as i64,
            mask.height().div_ceil(ts) as i64,
        );
        // 見えているタイルの範囲（表示の矩形の 4 隅を画布へ写して囲む）
        let clip = painter.clip_rect();
        let corners = [
            clip.left_top(),
            clip.right_top(),
            clip.right_bottom(),
            clip.left_bottom(),
        ]
        .map(|p| view.to_canvas(p));
        let (mut min_x, mut max_x, mut min_y, mut max_y) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
        for (x, y) in corners {
            min_x = min_x.min(x);
            max_x = max_x.max(x);
            min_y = min_y.min(y);
            max_y = max_y.max(y);
        }
        let tile = ts as f64;
        let tx0 = ((min_x / tile).floor() as i64).clamp(0, cols - 1);
        let tx1 = ((max_x / tile).floor() as i64).clamp(0, cols - 1);
        let ty0 = ((min_y / tile).floor() as i64).clamp(0, rows - 1);
        let ty1 = ((max_y / tile).floor() as i64).clamp(0, rows - 1);
        let visible = |c: &TileCoord| {
            (c.x as i64) >= tx0 && (c.x as i64) <= tx1 && (c.y as i64) >= ty0 && (c.y as i64) <= ty1
        };
        let coords: Vec<TileCoord> = mask.tile_coords().into_iter().filter(visible).collect();

        let same_mask = self.last.as_ref().is_some_and(|l| l.same_as(mask));
        let same_tint = self.tint == Some(tint);
        let stale = !same_mask || !same_tint;
        if stale {
            // 見えなくなった・量の無くなったタイルの絵は捨てる（見えるようになったら作る）
            let keep: HashSet<TileCoord> = coords.iter().copied().collect();
            self.slots.retain(|c, _| keep.contains(c));
        }
        let full = stale && (changed.is_none() || !same_tint);
        let mut buf = vec![0u8; (ts * ts) as usize];
        for c in &coords {
            let need = match self.slots.get(c) {
                None => true,
                Some(_) => full || (stale && changed.is_some_and(|h| h.contains(c))),
            };
            if need {
                self.rebuild(painter.ctx(), mask, *c, tint, &mut buf, name);
            }
        }
        self.last = Some(mask.clone());
        self.tint = Some(tint);

        // 描く
        let (w, h) = (mask.width() as f64, mask.height() as f64);
        for c in &coords {
            let (x0, y0) = (c.x as f64 * tile, c.y as f64 * tile);
            let Some(slot) = self.slots.get(c) else {
                continue;
            };
            match slot {
                Slot::Solid(v) => {
                    // 画布の外の余白は描かない
                    let corners = [
                        (x0, y0),
                        ((x0 + tile).min(w), y0),
                        ((x0 + tile).min(w), (y0 + tile).min(h)),
                        (x0, (y0 + tile).min(h)),
                    ];
                    let color = tinted(tint, *v);
                    let mut mesh = Mesh::default();
                    for (x, y) in corners {
                        mesh.vertices.push(Vertex {
                            pos: view.to_screen(x, y),
                            uv: WHITE_UV,
                            color,
                        });
                    }
                    mesh.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);
                    painter.add(Shape::mesh(mesh));
                }
                Slot::Texture { handle, .. } => {
                    CanvasDisplay::quad(
                        painter,
                        handle.id(),
                        view,
                        [
                            (x0, y0),
                            (x0 + tile, y0),
                            (x0 + tile, y0 + tile),
                            (x0, y0 + tile),
                        ],
                        [
                            pos2(0.0, 0.0),
                            pos2(1.0, 0.0),
                            pos2(1.0, 1.0),
                            pos2(0.0, 1.0),
                        ],
                    );
                }
            }
        }
    }

    fn rebuild(
        &mut self,
        ctx: &egui::Context,
        mask: &SelectionMask,
        coord: TileCoord,
        tint: Tint,
        buf: &mut [u8],
        name: &str,
    ) {
        if mask.copy_tile(coord, buf).is_err() {
            self.slots.remove(&coord);
            return;
        }
        let first = buf[0];
        if first != 0 && buf.iter().all(|&a| a == first) {
            self.slots.insert(coord, Slot::Solid(first));
            return;
        }
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        hasher.write(buf);
        let hash = hasher.finish();
        if let Some(Slot::Texture { hash: old, .. }) = self.slots.get(&coord) {
            if *old == hash {
                return;
            }
        }
        if self.texture_count() >= MAX_TEXTURES && !self.slots.contains_key(&coord) {
            return;
        }
        let ts = mask.tile_size() as usize;
        let pixels: Vec<Color32> = buf.iter().map(|&a| tinted(tint, a)).collect();
        let image = ColorImage::new([ts, ts], pixels);
        self.uploads += 1;
        match self.slots.get_mut(&coord) {
            Some(Slot::Texture { hash: h, handle }) => {
                handle.set(image, TextureOptions::NEAREST);
                *h = hash;
            }
            _ => {
                let handle = ctx.load_texture(
                    format!("{name}-{}-{}", coord.x, coord.y),
                    image,
                    TextureOptions::NEAREST,
                );
                self.slots.insert(coord, Slot::Texture { hash, handle });
            }
        }
    }
}

/// 量 `a`（0〜255）の濃さの色（乗算済み）。
fn tinted(tint: Tint, a: u8) -> Color32 {
    let alpha = (tint.alpha.clamp(0.0, 1.0) * a as f32).round() as u32;
    let [r, g, b] = tint.rgb.map(|c| ((c as u32 * alpha + 127) / 255) as u8);
    Color32::from_rgba_premultiplied(r, g, b, alpha as u8)
}
