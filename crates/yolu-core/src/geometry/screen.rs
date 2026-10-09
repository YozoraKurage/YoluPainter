//! 3D ビューの画面の形（グラデーション・図形の塗り）を、カメラから見える面のテクセルへ写す: 画面の形の覆いを、投影の塗り
//! （[`SurfaceProjector`]）の投影の画素ごとに測り、文書の画素ごとの量（選択範囲の形。0〜255）と、求めれば画素ごとの画面の位置にする
//! （文書には書かない。量を範囲にして塗るのは文書の塗りつぶし・グラデーション）。
//!
//! - 見え方は 3D のストロークと同じ投影の塗りの切り替え（隠れた所・裏の面・面の向きの弱め・継ぎ目のにじみ。どれを入れるかは呼ぶ側）。
//!   量は形の覆い × 面の向きの弱め。同じテクセルが何度も写る（重なった UV・にじみ）ときは量の大きい方、同じ量なら画面の位置の上
//!   （y の小さい方）、続けて左の方（区画を作る順・並列の度合いによらない）。
//! - 形の縁: 長方形（角の丸み）と楕円は、2D の図形の塗り（選択範囲の楕円・多角形）と同じく、1 テクセルを 4 × 4 の点で見て、形の中
//!   （縁の上を含む）にある点の数を量にする（0〜255 への丸めも同じ）。テクセルの中の点は、その投影の画素でのテクセル → 画面の写し
//!   （テクセルの x・y の 1 つ分の画面の動き）で画面へ写す（写しはテクセルの中心の一次の近似）。写しが使えない所は中心の 1 点。
//! - 集める箱: 形の箱に、テクセルの中の点が中心から離れる長さの 2 倍の余白を付ける。その長さは、形の中心（無ければ箱の辺の中点）の
//!   下のテクセルの写しで見積もり、どれもモデルに当たらなければテクセル 1 つ = 画面の 1 点とする。見積もりより大きく写るテクセルが
//!   箱の縁の外にあると、その形にかかる一部の点を数えない（箱の外の区画を作らないので）。
//! - 表示域の全体（グラデーション）には形の縁が無く、量は面の向きの弱めだけ。
//! - メモリ: 区画は覚えずに組ごとに作って捨て、一覧（共有・区画の三角形）・作った区画・候補・溜めた量（と画面の位置）の合計を、
//!   渡したバイト（1 回の操作の予算）に収める。超えたら [`DabRefusal::MemoryBudget`]。

use std::sync::Arc;

use glam::{DVec2, Vec2};

use super::build::FastMap;
use super::camera::CameraView;
use super::dab::DabRefusal;
use super::paint::{pick, SurfaceStrokeError};
use super::project::{AreaCover, ProjectionSettings, SurfaceProjector, SweepPixel};
use super::SurfaceGeometry;
use crate::{Document, SelectionMask, TileCoord};

/// 画面の形（画面の点。表示域の左上が原点で、下が +y）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ScreenShape {
    /// 表示域の全体（縁なし）。
    View,
    /// 向かい合う角 a・b の長方形。角の丸み（画面の点）は短い辺の半分まで。
    Rectangle { a: DVec2, b: DVec2, corner: f64 },
    /// 向かい合う角 a・b の箱に内接する楕円。
    Ellipse { a: DVec2, b: DVec2 },
}

/// 画面の形を面へ写す設定。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScreenCoverSettings {
    /// 塗るテクスチャセット（None なら全部）。
    pub material: Option<i32>,
    pub projection: ProjectionSettings,
    pub shape: ScreenShape,
    /// テクセルごとの画面の位置も持つ（グラデーション）。
    pub positions: bool,
    /// 使ってよいバイト（一覧・区画・候補・溜めた量と位置の合計）。
    pub room: u64,
}

/// 写した結果: テクセルごとの量（文書の大きさとタイルの選択範囲）と、求めたならテクセルごとの画面の位置。
pub struct ScreenCoverage {
    mask: SelectionMask,
    tile: u32,
    columns: u32,
    positions: FastMap<u32, Box<[Vec2]>>,
    /// 集めた区画の数（試験用）。
    pub buckets: u64,
}

impl ScreenCoverage {
    /// テクセルごとの量（量 0 の所は塗らない）。
    pub fn mask(&self) -> &SelectionMask {
        &self.mask
    }

    /// 量のあるテクセルが無いか。
    pub fn is_empty(&self) -> bool {
        self.mask.is_empty()
    }

    /// テクセル (x, y) の画面の位置（位置を求めなかった・量の無いテクセルは None か意味の無い値。量のある所だけ読む）。
    pub fn screen(&self, x: u32, y: u32) -> Option<DVec2> {
        if self.tile == 0 {
            return None;
        }
        let key = (y / self.tile) * self.columns + x / self.tile;
        let tile = self.positions.get(&key)?;
        let i = ((y % self.tile) * self.tile + x % self.tile) as usize;
        tile.get(i).map(|p| p.as_dvec2())
    }
}

/// 形（中心からのずれで測る）。
#[derive(Clone, Copy, Debug)]
enum Outline {
    /// 縁なし（表示域の全体）。
    None,
    /// 半分の幅 half、角の丸み corner の長方形。
    Rectangle { half: DVec2, corner: f64 },
    /// 半軸 half の楕円。
    Ellipse { half: DVec2 },
}

impl Outline {
    /// 中心からのずれ (dx, dy) の点が形の中（縁の上を含む）か。
    #[inline]
    fn inside(&self, dx: f64, dy: f64) -> bool {
        match *self {
            Outline::None => true,
            Outline::Rectangle { half, corner } => {
                let r = corner.clamp(0.0, half.min_element());
                let q = DVec2::new(dx.abs(), dy.abs()) - (half - DVec2::splat(r));
                q.max(DVec2::ZERO).length() + q.x.max(q.y).min(0.0) <= r
            }
            Outline::Ellipse { half } => {
                let (u, v) = (dx / half.x, dy / half.y);
                u * u + v * v <= 1.0
            }
        }
    }
}

/// 1 テクセルを見る点の数（1 辺）と、中心からのずれ（テクセルの単位。2D の選択範囲の形と同じ）。
const SAMPLES: usize = 4;
const OFFSETS: [f64; SAMPLES] = [-0.375, -0.125, 0.125, 0.375];

/// 形の覆い: 1 テクセルを 4 × 4 の点で見た、形の中の点の割合。
#[derive(Clone, Copy, Debug)]
struct ShapeCover {
    outline: Outline,
}

impl AreaCover for ShapeCover {
    fn cover(&self, dx: f64, dy: f64, columns: [f32; 4]) -> f32 {
        let [ax, ay, bx, by] = columns.map(|v| v as f64);
        let usable = [ax, ay, bx, by].iter().all(|v| v.is_finite()) && ax * by - ay * bx != 0.0;
        if !usable {
            return if self.outline.inside(dx, dy) {
                1.0
            } else {
                0.0
            };
        }
        let mut inside = 0u32;
        for oy in OFFSETS {
            for ox in OFFSETS {
                if self
                    .outline
                    .inside(dx + ax * ox + bx * oy, dy + ay * ox + by * oy)
                {
                    inside += 1;
                }
            }
        }
        inside as f32 / (SAMPLES * SAMPLES) as f32
    }
}

/// 溜めの 1 枚（文書のタイルの大きさ）。
struct GatherTile {
    amounts: Vec<u8>,
    screen: Vec<Vec2>,
}

/// テクセルごとの量（と画面の位置）の溜め。
struct Gather {
    width: u32,
    height: u32,
    tile: u32,
    columns: u32,
    positions: bool,
    tiles: FastMap<u32, GatherTile>,
    bytes: u64,
}

impl Gather {
    fn tile_bytes(&self) -> u64 {
        let n = (self.tile as u64) * (self.tile as u64);
        n + if self.positions {
            n * std::mem::size_of::<Vec2>() as u64
        } else {
            0
        } + 64
    }

    fn add(&mut self, p: &SweepPixel) {
        let (x, y) = (p.x as u32, p.y as u32);
        if x >= self.width || y >= self.height {
            return;
        }
        let a = (255.0 * p.coverage.clamp(0.0, 1.0)).round() as u8;
        if a == 0 {
            return;
        }
        let key = (y / self.tile) * self.columns + x / self.tile;
        if !self.tiles.contains_key(&key) {
            let n = (self.tile * self.tile) as usize;
            self.bytes += self.tile_bytes();
            self.tiles.insert(
                key,
                GatherTile {
                    amounts: vec![0; n],
                    screen: if self.positions {
                        vec![Vec2::ZERO; n]
                    } else {
                        Vec::new()
                    },
                },
            );
        }
        let tile = self.tiles.get_mut(&key).expect("作った");
        let i = ((y % self.tile) * self.tile + x % self.tile) as usize;
        let old = tile.amounts[i];
        let take = a > old
            || (a == old && self.positions && {
                let q = tile.screen[i];
                p.screen
                    .y
                    .total_cmp(&q.y)
                    .then(p.screen.x.total_cmp(&q.x))
                    .is_lt()
            });
        if take {
            tile.amounts[i] = a;
            if self.positions {
                tile.screen[i] = p.screen;
            }
        }
    }
}

/// 画面の形を、カメラから見える塗るテクスチャセットの面のテクセルへ写す（文書は読むだけ: 大きさとタイルの大きさ）。
pub fn cover_screen(
    doc: &Document,
    geometry: Arc<SurfaceGeometry>,
    view: &CameraView,
    settings: &ScreenCoverSettings,
) -> Result<ScreenCoverage, SurfaceStrokeError> {
    let (width, height, tile) = (doc.width(), doc.height(), doc.tile_size());
    let columns = width.div_ceil(tile);
    let empty = || -> Result<ScreenCoverage, SurfaceStrokeError> {
        Ok(ScreenCoverage {
            mask: SelectionMask::empty(width, height, tile)?,
            tile,
            columns,
            positions: FastMap::default(),
            buckets: 0,
        })
    };
    let screen = DVec2::new(view.width as f64, view.height as f64);
    // 中心・箱の半分の幅・形
    let (center, half, outline) = match settings.shape {
        ScreenShape::View => (screen * 0.5, screen * 0.5, Outline::None),
        ScreenShape::Rectangle { a, b, corner } => {
            let Some((center, half)) = box_of(a, b, corner)? else {
                return empty();
            };
            let corner = corner.max(0.0);
            (center, half, Outline::Rectangle { half, corner })
        }
        ScreenShape::Ellipse { a, b } => {
            let Some((center, half)) = box_of(a, b, 0.0)? else {
                return empty();
            };
            (center, half, Outline::Ellipse { half })
        }
    };
    let edged = !matches!(outline, Outline::None);
    // 区画は 8〜64 の画面の点（形が小さければ小さく）
    let bucket_radius = (half.max_element() as f32).clamp(16.0, 128.0);
    let mut projector = SurfaceProjector::new(
        geometry.clone(),
        view,
        settings.material,
        width as i32,
        height as i32,
        bucket_radius,
        settings.projection,
    )
    .map_err(SurfaceStrokeError::Dab)?
    .with_texel_columns(edged);
    // 集める箱の余白: テクセルの中の点が中心から離れる長さの 2 倍。4 × 4 の点のいちばん外は中心から各軸 0.375 テクセルで、各軸の
    // 写しの長さはいちばん長く写る向きの長さ以下なので、0.75 × その長さで押さえる。形の中心（無ければ箱の辺の中点）の下のテクセルで
    // 見積もり、どれもモデルに当たらなければテクセル 1 つ = 画面の 1 点
    let margin = if edged {
        let probes = [
            center,
            center + DVec2::new(half.x * 0.5, 0.0),
            center - DVec2::new(half.x * 0.5, 0.0),
            center + DVec2::new(0.0, half.y * 0.5),
            center - DVec2::new(0.0, half.y * 0.5),
        ];
        let texel = probes
            .iter()
            .find_map(|p| {
                pick(&geometry, view, p.as_vec2())
                    .filter(|h| settings.material.is_none_or(|m| m == h.material))
                    .and_then(|h| projector.metric_at(&h))
            })
            .map_or(1.0, |m| m.eigen().0.sqrt());
        2.0 * 0.75 * texel + 1.0
    } else {
        1.0
    };
    let cover = ShapeCover { outline };
    let bounds = (
        center - half - DVec2::splat(margin),
        center + half + DVec2::splat(margin),
    );
    let mut gather = Gather {
        width,
        height,
        tile,
        columns,
        positions: settings.positions,
        tiles: FastMap::default(),
        bytes: 0,
    };
    projector
        .sweep(center, &cover, true, bounds, settings.room, |list| {
            for p in list {
                gather.add(p);
            }
            gather.bytes
        })
        .map_err(SurfaceStrokeError::Dab)?;
    let buckets = projector.stats().buckets_built;
    let mut amounts = Vec::with_capacity(gather.tiles.len());
    let mut positions = FastMap::default();
    for (key, t) in gather.tiles {
        let coord = TileCoord::new(key % columns, key / columns);
        if settings.positions {
            positions.insert(key, t.screen.into_boxed_slice());
        }
        amounts.push((coord, t.amounts));
    }
    Ok(ScreenCoverage {
        mask: SelectionMask::from_amount_tiles(width, height, tile, amounts)?,
        tile,
        columns,
        positions,
        buckets,
    })
}

/// 向かい合う角 a・b の箱の中心と半分の幅（面積の無い箱は None）。
fn box_of(a: DVec2, b: DVec2, corner: f64) -> Result<Option<(DVec2, DVec2)>, SurfaceStrokeError> {
    if !a.is_finite() || !b.is_finite() || !corner.is_finite() {
        return Err(SurfaceStrokeError::Dab(DabRefusal::InvalidArguments));
    }
    let (lo, hi) = (a.min(b), a.max(b));
    let half = (hi - lo) * 0.5;
    Ok((half.x > 0.0 && half.y > 0.0).then_some(((lo + hi) * 0.5, half)))
}
