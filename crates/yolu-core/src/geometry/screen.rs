//! 3D ビューの画面の形（グラデーション・図形の塗り）を、カメラから見える面のテクセルへ写す: 画面の形の覆いを、投影の塗り
//! （[`SurfaceProjector`]）の投影の画素ごとに測り、文書の画素ごとの量（選択範囲の形。0〜255）と、求めれば画素ごとの画面の位置にする
//! （文書には書かない。量を範囲にして塗るのは文書の塗りつぶし・グラデーション）。
//!
//! - 見え方は 3D のストロークと同じ投影の塗りの切り替え（隠れた所・裏の面・面の向きの弱め・継ぎ目のにじみ。どれを入れるかは呼ぶ側）。
//!   量は形の覆い × 面の向きの弱め。同じテクセルが何度も写る（重なった UV・にじみ）ときは量の大きい方、同じ量なら画面の位置の上
//!   （y の小さい方）、続けて左の方（区画を作る順・並列の度合いによらない）。
//! - 形の縁: 長方形（角の丸み）・楕円・多角形（なげなわ）は、2D の図形の塗りと選択範囲の楕円・多角形と同じく、1 テクセルを 4 × 4 の点で見て、形の中
//!   （縁の上を含む。多角形は偶奇の規則）にある点の数を量にする（0〜255 への丸めも同じ）。テクセルの中の点は、その投影の画素でのテクセル → 画面の写し
//!   （テクセルの x・y の 1 つ分の画面の動き）で画面へ写す（写しはテクセルの中心の一次の近似）。写しが使えない所は中心の 1 点。
//! - 集める箱: 形の箱に、テクセルの中の点が中心から離れる長さの 2 倍の余白を付ける。その長さは、形の中心（無ければ箱の辺の中点）の
//!   下のテクセルの写しで見積もり、どれもモデルに当たらなければテクセル 1 つ = 画面の 1 点とする（多角形は、箱の中の格子の点の下の
//!   テクセルのうち最も長く写るもの。形が曲がっていて中心が外れうるので）。見積もりより大きく写るテクセルが
//!   箱の縁の外にあると、その形にかかる一部の点を数えない（箱の外の区画を作らないので）。
//! - 多角形の中かどうかは、辺を画面の高さの帯ごとに分けておき、点の高さの帯の辺だけを数える（2D の `SelectionMask::polygon` の式のまま。
//!   点の多いなげなわでも、1 つの点の判定が全部の辺を辿らない）。帯の表は一覧と同じ予算に数える。
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
use crate::selection::MAX_POLYGON_POINTS;
use crate::{Document, SelectionMask, TileCoord};

/// 画面の形（画面の点。表示域の左上が原点で、下が +y）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ScreenShape<'a> {
    /// 表示域の全体（縁なし）。
    View,
    /// 向かい合う角 a・b の長方形。角の丸み（画面の点）は短い辺の半分まで。
    Rectangle { a: DVec2, b: DVec2, corner: f64 },
    /// 向かい合う角 a・b の箱に内接する楕円。
    Ellipse { a: DVec2, b: DVec2 },
    /// 頂点の並び（最後から最初へも結ぶ）の多角形。偶奇の規則で、2D の投げ縄・多角形（`SelectionMask::polygon`）と同じ。3 つ未満は何も覆わない。
    Polygon { points: &'a [DVec2] },
}

/// 画面の形を面へ写す設定。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScreenCoverSettings<'a> {
    /// 塗るテクスチャセット（None なら全部）。
    pub material: Option<i32>,
    pub projection: ProjectionSettings,
    pub shape: ScreenShape<'a>,
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
    /// 多角形の帯の表が持っていたバイト（多角形でなければ 0。試験用）。
    pub table_bytes: u64,
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
enum Outline<'a> {
    /// 縁なし（表示域の全体）。
    None,
    /// 半分の幅 half、角の丸み corner の長方形。
    Rectangle { half: DVec2, corner: f64 },
    /// 半軸 half の楕円。
    Ellipse { half: DVec2 },
    /// 多角形（辺は中心からのずれ）。
    Polygon(&'a PolygonEdges),
}

impl Outline<'_> {
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
            Outline::Polygon(edges) => edges.inside(dx, dy),
        }
    }
}

/// 帯の高さ（画面の点）。
const BAND: f64 = 1.0;
/// 帯に分ける範囲を、表示域の上下へ広げる余白（画面の点）。範囲の外の点は、全部の辺を辿る遅い道で判定する（結果は同じ）。
const BAND_MARGIN: f64 = 256.0;

/// 多角形の辺（中心からのずれ）と、高さの帯ごとの辺の番号。点 (dx, dy) の中かどうかは、2D の `SelectionMask::polygon` と同じ式（辺は
/// 点 i から 1 つ前の点 j へ、`(a.y > py) != (b.y > py)` をまたぎ、交点の x より左の点を右の交点の数の偶奇で数える）を、その高さの帯の辺だけで数える。
struct PolygonEdges {
    /// 水平でない辺 (a.x, a.y, b.x, b.y)。
    edges: Vec<[f64; 4]>,
    /// 帯の始まり（中心からのずれ）。
    top: f64,
    /// 帯ごとの辺の番号の範囲（`items` の中。帯の数 + 1 個）。
    starts: Vec<u32>,
    items: Vec<u32>,
    /// 全部の点の高さの範囲（この外は何も覆わない）。
    span: (f64, f64),
}

impl std::fmt::Debug for PolygonEdges {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PolygonEdges({} edges)", self.edges.len())
    }
}

impl PolygonEdges {
    /// 点 `points`（画面の点）の辺を、中心 `center` からのずれで持つ。`frame_height` は表示域の高さ。帯の表と辺の一覧が `room` バイトを超えるなら
    /// `MemoryBudget`。
    fn new(
        points: &[DVec2],
        center: DVec2,
        frame_height: f64,
        room: u64,
    ) -> Result<PolygonEdges, DabRefusal> {
        let at = |i: usize| points[i] - center;
        let mut edges = Vec::with_capacity(points.len());
        let (mut lo, mut hi) = (f64::MAX, f64::MIN);
        let mut j = points.len() - 1;
        for i in 0..points.len() {
            let (a, b) = (at(i), at(j));
            lo = lo.min(a.y);
            hi = hi.max(a.y);
            if a.y != b.y {
                edges.push([a.x, a.y, b.x, b.y]);
            }
            j = i;
        }
        // 帯に分ける範囲: 全部の点の高さと、表示域（の上下に余白）の重なり
        let band_lo = lo.max(-center.y - BAND_MARGIN);
        let band_hi = hi.min(frame_height - center.y + BAND_MARGIN);
        let bands = if band_hi >= band_lo {
            ((band_hi - band_lo) / BAND).floor() as usize + 1
        } else {
            0
        };
        let band_top = band_lo + bands as f64 * BAND;
        // 高さ y の帯の番号（範囲の外は両端の帯へ寄せる。辺の端を寄せても、その帯に余計な辺が入るだけで、足りなくはならない）
        let band_of = |y: f64| {
            ((y - band_lo) / BAND)
                .floor()
                .clamp(0.0, (bands - 1) as f64) as usize
        };
        // 辺がまたぐ帯（またがない・範囲の外なら None）
        let span_of = |e: &[f64; 4]| -> Option<(usize, usize)> {
            let (y0, y1) = (e[1].min(e[3]), e[1].max(e[3]));
            (bands > 0 && y1 >= band_lo && y0 <= band_top).then(|| (band_of(y0), band_of(y1)))
        };
        // 帯ごとの辺の数（差分で数えて、辺が長くても辺の数と帯の数に比例する時間で見積もる）
        let mut diff = vec![0i64; bands + 1];
        for e in &edges {
            if let Some((first, last)) = span_of(e) {
                diff[first] += 1;
                diff[last + 1] -= 1;
            }
        }
        let mut starts = vec![0u32; bands + 1];
        let (mut running, mut total) = (0i64, 0u64);
        for k in 0..bands {
            running += diff[k];
            total += running as u64;
            starts[k + 1] = total.min(u32::MAX as u64) as u32;
        }
        let bytes = edges.len() as u64 * std::mem::size_of::<[f64; 4]>() as u64
            + (bands as u64 + 1) * 12
            + total * 4;
        if bytes > room || total > u32::MAX as u64 {
            return Err(DabRefusal::MemoryBudget);
        }
        let mut fill: Vec<u32> = starts[..bands].to_vec();
        let mut items = vec![0u32; total as usize];
        for (index, e) in edges.iter().enumerate() {
            if let Some((first, last)) = span_of(e) {
                for slot in &mut fill[first..=last] {
                    items[*slot as usize] = index as u32;
                    *slot += 1;
                }
            }
        }
        Ok(PolygonEdges {
            edges,
            top: band_lo,
            starts,
            items,
            span: (lo, hi),
        })
    }

    /// 持っているバイト（辺・帯の始まり・帯ごとの辺の番号）。
    fn bytes(&self) -> u64 {
        (self.edges.len() * std::mem::size_of::<[f64; 4]>()
            + (self.starts.len() + self.items.len()) * std::mem::size_of::<u32>()) as u64
    }

    /// 高さ dy の所にある辺のうち、x が dx より右で交わるものの数の偶奇。
    #[inline]
    fn inside(&self, dx: f64, dy: f64) -> bool {
        if !(dy >= self.span.0 && dy <= self.span.1) {
            return false;
        }
        let bands = self.starts.len() - 1;
        let k = ((dy - self.top) / BAND).floor();
        let mut right = 0u32;
        let mut count = |e: &[f64; 4]| {
            let [ax, ay, bx, by] = *e;
            if (ay > dy) != (by > dy) {
                let xc = (bx - ax) * (dy - ay) / (by - ay) + ax;
                // dx < NaN は偽なので、NaN は数えない
                if dx < xc {
                    right += 1;
                }
            }
        };
        if k >= 0.0 && (k as usize) < bands {
            let k = k as usize;
            for &i in &self.items[self.starts[k] as usize..self.starts[k + 1] as usize] {
                count(&self.edges[i as usize]);
            }
        } else {
            // 帯に分けた範囲の外: 全部の辺を辿る（まれ）
            self.edges.iter().for_each(count);
        }
        right % 2 == 1
    }
}

/// 1 テクセルを見る点の数（1 辺）と、中心からのずれ（テクセルの単位。2D の選択範囲の形と同じ）。
const SAMPLES: usize = 4;
const OFFSETS: [f64; SAMPLES] = [-0.375, -0.125, 0.125, 0.375];

/// 形の覆い: 1 テクセルを 4 × 4 の点で見た、形の中の点の割合。
#[derive(Clone, Copy, Debug)]
struct ShapeCover<'a> {
    outline: Outline<'a>,
}

impl AreaCover for ShapeCover<'_> {
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
    settings: &ScreenCoverSettings<'_>,
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
            table_bytes: 0,
        })
    };
    let screen = DVec2::new(view.width as f64, view.height as f64);
    // 多角形の辺（形の中心からのずれ。`outline` が借りるので、先に置き場を作る）
    let polygon;
    let mut table_bytes = 0;
    // 区画・候補・溜めに使ってよいバイト（多角形の帯の表を引いた残り）
    let mut room = settings.room;
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
        ScreenShape::Polygon { points } => {
            let Some((center, half)) = box_of_points(points)? else {
                return empty();
            };
            polygon = PolygonEdges::new(points, center, screen.y, settings.room)
                .map_err(SurfaceStrokeError::Dab)?;
            table_bytes = polygon.bytes();
            room = room.saturating_sub(table_bytes);
            (center, half, Outline::Polygon(&polygon))
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
        let texel_at = |p: DVec2| {
            pick(&geometry, view, p.as_vec2())
                .filter(|h| settings.material.is_none_or(|m| m == h.material))
                .and_then(|h| projector.metric_at(&h))
                .map(|m| m.eigen().0.sqrt())
        };
        let texel = match settings.shape {
            // 多角形は形が曲がっていて、箱の中心がモデルの外のこともある。箱の中の格子の点の下で、いちばん長く写るテクセルで見積もる
            ScreenShape::Polygon { .. } => {
                const GRID: [f64; 5] = [-0.8, -0.4, 0.0, 0.4, 0.8];
                GRID.iter()
                    .flat_map(|&gy| GRID.iter().map(move |&gx| (gx, gy)))
                    .filter_map(|(gx, gy)| texel_at(center + half * DVec2::new(gx, gy)))
                    .fold(None, |best: Option<f64>, t| {
                        Some(best.map_or(t, |b| b.max(t)))
                    })
            }
            _ => [
                center,
                center + DVec2::new(half.x * 0.5, 0.0),
                center - DVec2::new(half.x * 0.5, 0.0),
                center + DVec2::new(0.0, half.y * 0.5),
                center - DVec2::new(0.0, half.y * 0.5),
            ]
            .iter()
            .find_map(|&p| texel_at(p)),
        }
        .unwrap_or(1.0);
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
        .sweep(center, &cover, true, bounds, room, |list| {
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
        table_bytes,
    })
}

/// 点の並びを囲む箱の中心と半分の幅（3 点に満たない・面積の無い箱は None。点が多すぎる・有限でないなら誤り）。
fn box_of_points(points: &[DVec2]) -> Result<Option<(DVec2, DVec2)>, SurfaceStrokeError> {
    if points.len() > MAX_POLYGON_POINTS || points.iter().any(|p| !p.is_finite()) {
        return Err(SurfaceStrokeError::Dab(DabRefusal::InvalidArguments));
    }
    if points.len() < 3 {
        return Ok(None);
    }
    let (lo, hi) = points.iter().fold((DVec2::MAX, DVec2::MIN), |(lo, hi), p| {
        (lo.min(*p), hi.max(*p))
    });
    let half = (hi - lo) * 0.5;
    Ok((half.x > 0.0 && half.y > 0.0).then_some(((lo + hi) * 0.5, half)))
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 全部の辺を辿る判定（2D の `SelectionMask::polygon` と同じ式）。
    fn brute(points: &[DVec2], p: DVec2) -> bool {
        let mut right = 0;
        let mut j = points.len() - 1;
        for i in 0..points.len() {
            let (a, b) = (points[i], points[j]);
            if (a.y > p.y) != (b.y > p.y) {
                let xc = (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x;
                if p.x < xc {
                    right += 1;
                }
            }
            j = i;
        }
        right % 2 == 1
    }

    /// 決まった並びの疑似乱数（0〜1）。
    fn next(state: &mut u64) -> f64 {
        *state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (*state >> 11) as f64 / (1u64 << 53) as f64
    }

    #[test]
    fn the_banded_test_gives_the_same_answer_as_walking_every_edge_inside_and_outside_the_banded_range(
    ) {
        let mut seed = 7u64;
        for n in [3usize, 4, 9, 40, 300] {
            // 表示域（高さ 200）の上下に大きくはみ出す点も混ぜる
            let points: Vec<DVec2> = (0..n)
                .map(|i| {
                    let far = i % 7 == 3;
                    DVec2::new(
                        next(&mut seed) * 260.0 - 30.0,
                        if far {
                            next(&mut seed) * 4000.0 - 2000.0
                        } else {
                            next(&mut seed) * 260.0 - 30.0
                        },
                    )
                })
                .collect();
            let (lo, hi) = points
                .iter()
                .fold((DVec2::MAX, DVec2::MIN), |(l, h), p| (l.min(*p), h.max(*p)));
            let center = (lo + hi) * 0.5;
            let edges = PolygonEdges::new(&points, center, 200.0, 1 << 30).unwrap();
            for _ in 0..4000 {
                // 点は箱の中と、箱の少し外（帯の範囲の外・全部の点の高さの外）から
                let p = DVec2::new(
                    lo.x - 20.0 + next(&mut seed) * (hi.x - lo.x + 40.0),
                    lo.y - 20.0 + next(&mut seed) * (hi.y - lo.y + 40.0),
                );
                assert_eq!(
                    edges.inside(p.x - center.x, p.y - center.y),
                    brute(&points, p),
                    "{n} 点 {p:?}"
                );
            }
        }
    }
}
