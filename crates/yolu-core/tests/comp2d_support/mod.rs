//! 2D の合成の速さの試験が共有する、乱数の文書の組み立て（`mod comp2d_support;` で読む）。
#![allow(dead_code)]

use yolu_core::{
    AdjustmentSettings, BlendMode, Channel, Document, LayerId, LayerKind, Rgba8, TileCoord,
};

pub struct Rng(pub u64);
impl Rng {
    pub fn bits(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    }
    pub fn below(&mut self, n: u64) -> u64 {
        self.bits() % n.max(1)
    }
    pub fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }
    pub fn color(&mut self) -> Rgba8 {
        let r = self.bits();
        let a = match r >> 56 & 7 {
            0 => 0,
            1 | 2 => 255,
            _ => (r >> 24) as u8,
        };
        Rgba8::new(r as u8, (r >> 8) as u8, (r >> 16) as u8, a)
    }
}

/// レイヤーに長方形と散らばった点で画素を置く（Normal のチャンネルにも少し）。
fn paint(d: &mut Document, rng: &mut Rng, id: LayerId, normal: bool) {
    let (w, h) = (d.width() as u64, d.height() as u64);
    for _ in 0..1 + rng.below(3) {
        let (x0, y0) = (rng.below(w), rng.below(h));
        let (x1, y1) = (
            (x0 + 1 + rng.below(w / 2 + 1)).min(w),
            (y0 + 1 + rng.below(h / 2 + 1)).min(h),
        );
        let c = rng.color();
        for y in y0..y1 {
            for x in x0..x1 {
                d.set_pixel(id, x as u32, y as u32, c).unwrap();
            }
        }
    }
    for _ in 0..rng.below(120) {
        let c = rng.color();
        let (x, y) = (rng.below(w) as u32, rng.below(h) as u32);
        d.set_pixel(id, x, y, c).unwrap();
        if normal {
            d.set_channel_pixel(id, Channel::Normal, x, y, c).unwrap();
        }
    }
}

/// 乱数の文書: ラスターレイヤー（合成モード・不透明度・クリッピング・マスク）・調整・塗りつぶし・入れ子のグループ（通過と分離）。
/// 返す 2 つ目は、ラスターレイヤーの ID（描くレイヤーの候補。下から上）。
pub fn random_doc(seed: u64, width: u32, height: u32, tile: u32) -> (Document, Vec<LayerId>) {
    let mut rng = Rng(seed);
    let mut d = Document::with_tile_size(width, height, tile).unwrap();
    let mut rasters = Vec::new();
    let mut order: Vec<LayerId> = Vec::new(); // 作った順（グループにまとめる候補）
    let count = 6 + rng.below(9);
    for i in 0..count {
        match rng.below(100) {
            0..=69 => {
                let id = d.add_layer(&format!("r{i}")).unwrap();
                let normal = rng.chance(30);
                paint(&mut d, &mut rng, id, normal);
                if rng.chance(70) {
                    let m = BlendMode::LAYER_MODES[rng.below(26) as usize];
                    d.set_layer_blend_mode(id, m).unwrap();
                }
                if rng.chance(70) {
                    d.set_layer_opacity(id, 0.2 + rng.below(81) as f64 / 100.0, false)
                        .unwrap();
                }
                if !order.is_empty() && rng.chance(25) {
                    d.set_layer_clipping(id, true).unwrap();
                }
                if rng.chance(20) {
                    d.add_layer_mask(id).unwrap();
                    for _ in 0..rng.below(200) {
                        let (x, y) = (
                            rng.below(width as u64) as u32,
                            rng.below(height as u64) as u32,
                        );
                        d.set_mask_pixel(id, x, y, rng.below(256) as u8).unwrap();
                    }
                }
                rasters.push(id);
                order.push(id);
            }
            70..=79 => {
                let above = order.last().copied();
                let s = AdjustmentSettings::levels(
                    0.02 * rng.below(5) as f64,
                    1.0 - 0.02 * rng.below(5) as f64,
                    0.7 + 0.1 * rng.below(8) as f64,
                    0.0,
                    1.0,
                )
                .unwrap();
                let id = d
                    .add_adjustment_layer(&format!("a{i}"), s, None, above)
                    .unwrap();
                if rng.chance(50) {
                    d.set_layer_opacity(id, 0.3 + rng.below(70) as f64 / 100.0, false)
                        .unwrap();
                }
                order.push(id);
            }
            80..=87 => {
                let color = rng.color();
                let id = d
                    .add_fill_layer(&format!("f{i}"), &[(Channel::Color, color)], None)
                    .unwrap();
                if rng.chance(60) {
                    d.set_layer_opacity(id, 0.1 + rng.below(60) as f64 / 100.0, false)
                        .unwrap();
                }
                if rng.chance(50) {
                    let m = BlendMode::LAYER_MODES[rng.below(26) as usize];
                    d.set_layer_blend_mode(id, m).unwrap();
                }
                order.push(id);
            }
            _ => {
                // 直前の数枚をグループにまとめる（通過か分離かは、モードで決める）
                let n = 2 + rng.below(3) as usize;
                if order.len() >= n {
                    let from = order.len() - n;
                    let members: Vec<LayerId> = order[from..].to_vec();
                    // 別のグループの中のレイヤーは、同じ親のものだけをまとめる
                    let parent = d.layer(members[0]).unwrap().parent();
                    if members
                        .iter()
                        .all(|m| d.layer(*m).unwrap().parent() == parent)
                    {
                        if let Ok(g) = d.group_layers(&members, &format!("g{i}")) {
                            if rng.chance(50) {
                                let m = BlendMode::LAYER_MODES[rng.below(26) as usize];
                                d.set_layer_blend_mode(g, m).unwrap();
                            }
                            if rng.chance(50) {
                                d.set_layer_opacity(g, 0.3 + rng.below(70) as f64 / 100.0, false)
                                    .unwrap();
                            }
                            order.truncate(from);
                            order.push(g);
                        }
                    }
                }
            }
        }
    }
    d.clear_history().unwrap();
    (d, rasters)
}

/// ラスターレイヤーの ID のうち、面を持つもの。
pub fn paintable(d: &Document, rasters: &[LayerId]) -> Vec<LayerId> {
    rasters
        .iter()
        .copied()
        .filter(|id| {
            d.layer(*id).is_some_and(|l| {
                l.kind() == LayerKind::Raster && l.surface(Channel::Color).is_some()
            })
        })
        .collect()
}

pub fn all_tiles(d: &Document) -> Vec<TileCoord> {
    d.canvas_tiles().collect()
}
