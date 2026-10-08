//! UV の位相（[`UvTopology`]）: テクスチャセットの三角形の UV アイランド・アイランドの図（テクセル → アイランド。[`IslandMap`]）・アイランドの縁の 3D の相手
//! （UV の継ぎ目）。レイヤーのフィルターが継ぎ目をまたいで読む帯の写し（[`SeamBand`](super::SeamBand)）と、UV アイランドごとの効果が読むアイランドの番号の元。
//!
//! - **アイランド**は、UV で辺を共有してつながる三角形（`regions` の UV アイランドと同じ決まり: UV を 1e-6 で量子化し、同じスロットの中だけ）。
//!   番号は 1 から、アイランドのいちばん小さい三角形の番号の順（モデルが同じなら、解像度によらず同じ番号）。0 はアイランドの外。UV が重なったアイランド
//!   （ミラーで両側が同じ UV を使うなど）は、辺を共有していれば 1 つのアイランドになる。
//! - **アイランドの図**は、テクセルの中心を、ベイクの割り当て（`mesh_maps` の行の割り当てを 1 テクセル 1 点で）で三角形に割り当て、その三角形のアイランドに
//!   したもの。2 つ以上の三角形の内側が覆うテクセルは「重なり」の印（ベイクの重なりと同じ見つけ方）。行ごとの連なりで持つ。
//! - **継ぎ目の縁**は、3D で辺を共有する（溶接した位置が同じ 2 頂点。`SurfaceGeometry` の隣り合わせ）のに UV が違う辺。相手の無い縁
//!   （開いた縁・3 つ以上の三角形が使う辺）は継ぎ目にしない。
//! - アイランド・縁の対応・アイランドの図・帯の写しは、初めて要るときに作る。アイランドの図は解像度ごと、帯の写しは解像度と帯の幅ごとに覚える（新しい 2 つずつ）。
//!   位置は作ったときのスナップショットのもの。
//! - アイランドの図・帯の写しは、呼ぶ側が渡す**作業予算**（バイト）の中で作る（`island_map_within`・`seam_band_within`）: 作る途中の確保の見積りが予算を
//!   超えれば作らずに断り（[`UvTopologyError::Budget`]）、作ったら、覚えているアイランドの図と帯の写しの合計が予算に収まるよう古いものから捨てる
//!   （直近の 1 つずつは残す）。断った大きさと予算は覚え、同じか小さい予算での頼みは作り直さずに断る（[`UvTopology::refusal`]）。同じ大きさを
//!   別のスレッドが同時に頼んでも、作るのは 1 回（待たせる）。
//! - 作るのは、rayon のスレッドの外（アプリの主のスレッドや裏の仕事）からなら rayon で並列に、rayon のスレッドの中（評価のブロックを並べている最中など）からなら
//!   並列にせず今のスレッドで順に回す（[`may_parallelize`]。結果は同じ）。作る間は門・`OnceLock` を持っていて、並列の仕事の終わりを待つスレッドは同じスレッドの
//!   中で別の仕事を拾うので、拾った仕事が同じ物を取りに来ると、自分が持つ物を待って止まるから。評価の側は、並列の仕事を並べる前に要る物を作っておく。

use std::collections::hash_map::Entry;
use std::sync::{Arc, Mutex, OnceLock};

use glam::Vec2;
use rayon::prelude::*;

use super::build::{position_key, FastMap};
use super::seam_band::SeamBand;
use super::SurfaceGeometry;
use crate::mesh_maps::raster::Raster;

/// アイランドの図・帯の写しの辺の上限（文書の辺の上限と同じ）。
pub const MAX_TOPOLOGY_EDGE: u32 = 8192;
/// 帯の幅の上限（フィルターのスタックの半径の合計の上限と同じ）。
pub const MAX_SEAM_BAND: u32 = crate::filter::MAX_HALO;
/// 覚えておくアイランドの図・帯の写しの数（それぞれ）。
const KEEP: usize = 2;
/// 作業予算を渡さない `island_map`・`seam_band` と、文書のアイランドの図・帯の写しの予算の既定（1 GiB）。アイランドの図と帯の写しの大きさは、モデルの
/// 三角形の数・アイランドの間の隙間・解像度で決まる（4096² で数十 MiB、8192² の密な並びで数百 MiB）。上限は、それを超える並びで確保を止めるためのもの。
pub const DEFAULT_BUDGET: u64 = 1024 * 1024 * 1024;
/// 断った作り方として覚える数。
const REFUSALS: usize = 8;

/// アイランドの図・帯の写しを作れなかった理由。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UvTopologyError {
    /// 解像度か帯の幅が範囲の外。
    InvalidSize,
    /// 行の割り当てが断った（理由）。
    Raster(String),
    /// アイランドの図・帯の写しを作る作業メモリが、渡された予算（バイト）に収まらない。
    Budget { budget: u64 },
}

impl std::fmt::Display for UvTopologyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UvTopologyError::InvalidSize => f.write_str("UV アイランドの図の大きさが範囲の外です"),
            UvTopologyError::Raster(why) => write!(f, "UV アイランドの図を作れません（{why}）"),
            UvTopologyError::Budget { budget } => {
                let size = if *budget >= 1 << 20 {
                    format!("{} MiB", budget.div_ceil(1 << 20))
                } else if *budget >= 1 << 10 {
                    format!("{} KiB", budget.div_ceil(1 << 10))
                } else {
                    format!("{budget} バイト")
                };
                write!(
                    f,
                    "UV アイランドの図・帯の写しを作る作業メモリが予算（{size}）に収まりません"
                )
            }
        }
    }
}

impl std::error::Error for UvTopologyError {}

/// アイランドの図の、行の中の連なり 1 つ（`start..end` のテクセルが同じアイランド）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IslandRun {
    pub start: u32,
    pub end: u32,
    /// アイランドの番号（1 から）。
    pub island: u32,
    /// 2 つ以上の三角形の内側が覆う（重なった UV）。`island` は番号の小さい三角形のアイランド。
    pub overlap: bool,
}

/// アイランドの図: テクセル → UV アイランドの番号（左下原点）。行ごとの連なりで持つ。
#[derive(Debug, PartialEq)]
pub struct IslandMap {
    width: u32,
    height: u32,
    island_count: u32,
    /// 行 y の連なりは `runs[rows[y]..rows[y + 1]]`（x の順、重ならない）。
    rows: Vec<u32>,
    runs: Vec<IslandRun>,
}

impl IslandMap {
    pub fn width(&self) -> u32 {
        self.width
    }
    pub fn height(&self) -> u32 {
        self.height
    }
    /// モデルのそのセットのアイランドの数（番号は 1..=数）。図に出ない小さなアイランドも数える。
    pub fn island_count(&self) -> u32 {
        self.island_count
    }
    /// 行 y の連なり（x の順。間のテクセルはアイランドの外）。
    pub fn row(&self, y: u32) -> &[IslandRun] {
        let y = y as usize;
        &self.runs[self.rows[y] as usize..self.rows[y + 1] as usize]
    }
    fn run(&self, x: u32, y: u32) -> Option<&IslandRun> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let row = self.row(y);
        let i = row.partition_point(|r| r.end <= x);
        row.get(i).filter(|r| r.start <= x)
    }
    /// テクセル (x, y) のアイランドの番号（アイランドの外・図の外は 0）。
    pub fn island(&self, x: u32, y: u32) -> u32 {
        self.run(x, y).map_or(0, |r| r.island)
    }
    /// テクセル (x, y) を 2 つ以上の三角形の内側が覆うか。
    pub fn overlapped(&self, x: u32, y: u32) -> bool {
        self.run(x, y).is_some_and(|r| r.overlap)
    }
    /// アイランドの中のテクセルの数。
    pub fn covered_texels(&self) -> u64 {
        self.runs.iter().map(|r| u64::from(r.end - r.start)).sum()
    }
    /// 持っているバイト数（名目）。
    pub fn bytes(&self) -> u64 {
        (self.rows.len() * 4 + self.runs.len() * std::mem::size_of::<IslandRun>()) as u64
    }
}

/// 継ぎ目の縁 1 つ（三角形の辺と、3D で同じ辺を持つ相手の三角形）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SeamEdge {
    pub triangle: u32,
    /// 辺の始まりの頂点（辺は `edge` → `edge + 1`）。
    pub edge: u8,
    pub partner: u32,
    /// 相手の三角形の頂点のうち、こちらの `edge`・`edge + 1` と同じ位置のもの。
    pub partner_a: u8,
    pub partner_b: u8,
    /// アイランドの縁をたどって、この辺の前の縁の始まりと、後の縁の終わり（UV）。縁が一筋にたどれない所（重なった UV・向きの揃わないアイランド）は None。
    pub prev: Option<Vec2>,
    pub next: Option<Vec2>,
}

/// 三角形の辺の向こう（3D で同じ 2 頂点を持つ隣）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Link {
    /// 隣の三角形（無ければ `u32::MAX`）。
    pub neighbor: u32,
    /// 隣の三角形の頂点のうち、こちらの辺の始まり・終わりと同じ位置のもの。
    pub a: u8,
    pub b: u8,
    /// UV もつながる（アイランドの中の辺）。
    pub continuous: bool,
}

impl Link {
    const NONE: Link = Link {
        neighbor: u32::MAX,
        a: 0,
        b: 0,
        continuous: false,
    };
}

/// アイランドと縁の対応（解像度によらない）。
#[derive(Debug, PartialEq)]
pub(crate) struct Parts {
    /// 三角形の番号ごとのアイランド（セットの外は 0）。
    pub island_of: Vec<u32>,
    pub island_count: u32,
    /// 継ぎ目の縁（三角形の番号・辺の順）。
    pub seams: Vec<SeamEdge>,
    /// 三角形の辺（頂点 j → j + 1）ごとの向こう（セットの三角形だけ）。
    pub links: Vec<[Link; 3]>,
}

/// モデルのテクスチャセット 1 つの UV の位相。作るのは安く（写しは持たない）、重い物は初めて要るときに作る。`Send + Sync`。
pub struct UvTopology {
    geometry: Arc<SurfaceGeometry>,
    material: Option<i32>,
    /// 初めて要るときに作る（作る間、ほかのスレッドは `get_or_init` で待つ）。持ったまま rayon の仕事を待たない: 作るのが rayon のスレッドの中なら
    /// 並列にしない（[`may_parallelize`]）。
    parts: OnceLock<Parts>,
    maps: Mutex<Vec<Arc<IslandMap>>>,
    bands: Mutex<Vec<Arc<SeamBand>>>,
    /// 作業予算で断った作り方（新しいものが後ろ）。
    refused: Mutex<Vec<Refusal>>,
    /// アイランドの図・帯の写しを作る間の門（同じものを同時に作らない）。帯の写しの門を持ってからアイランドの図の門を取る。
    /// 門を持ったまま rayon の仕事を待たない: 待つ間のスレッドは同じスレッドの中で別の仕事（同じ門を取りに来る評価のブロックなど）を拾い、自分が持つ門を
    /// 待って止まる。作るのが rayon のスレッドの中なら並列にしない（[`may_parallelize`]）。
    map_gate: Mutex<()>,
    band_gate: Mutex<()>,
}

/// 作業予算で断った作り方: 大きさ・帯の幅（0 はアイランドの図）と、そのときの予算。
#[derive(Clone, Copy, Debug)]
struct Refusal {
    width: u32,
    height: u32,
    band: u32,
    budget: u64,
}

impl std::fmt::Debug for UvTopology {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UvTopology")
            .field("revision", &self.geometry.revision())
            .field("triangles", &self.geometry.triangle_count())
            .field("material", &self.material)
            .finish_non_exhaustive()
    }
}

impl UvTopology {
    /// モデルのスナップショットと、テクスチャセットのマテリアルの組（None はモデル全体）から。
    pub fn new(geometry: Arc<SurfaceGeometry>, material: Option<i32>) -> UvTopology {
        UvTopology {
            geometry,
            material,
            parts: OnceLock::new(),
            maps: Mutex::new(Vec::new()),
            bands: Mutex::new(Vec::new()),
            refused: Mutex::new(Vec::new()),
            map_gate: Mutex::new(()),
            band_gate: Mutex::new(()),
        }
    }

    pub fn geometry(&self) -> &Arc<SurfaceGeometry> {
        &self.geometry
    }

    /// 別のスナップショットが、同じ UV の位相か（三角形の並び・UV・スロット・マテリアルの組・隣り合わせが同じ。位置は見ない）。
    /// ポーズで位置だけが変わったモデルでは true（アイランド・継ぎ目・帯の写しを作り直さずに使い続けられる。展開は作ったときの位置のまま）。
    pub fn same_layout(&self, other: &SurfaceGeometry) -> bool {
        let g = &*self.geometry;
        g.triangles.len() == other.triangles.len()
            && g.adjacency_offsets == other.adjacency_offsets
            && g.adjacency == other.adjacency
            && g.triangles.iter().zip(&other.triangles).all(|(a, b)| {
                a.uv_a == b.uv_a
                    && a.uv_b == b.uv_b
                    && a.uv_c == b.uv_c
                    && a.material_slot == b.material_slot
                    && a.material == b.material
                    && a.renderer == b.renderer
            })
    }
    pub fn material(&self) -> Option<i32> {
        self.material
    }

    pub(crate) fn parts(&self) -> &Parts {
        self.parts
            .get_or_init(|| Parts::new(&self.geometry, self.material, may_parallelize()))
    }

    /// アイランドの数（番号は 1..=数）。
    pub fn island_count(&self) -> u32 {
        self.parts().island_count
    }
    /// 三角形のアイランドの番号（セットの外・範囲外は 0）。
    pub fn triangle_island(&self, triangle: u32) -> u32 {
        self.parts()
            .island_of
            .get(triangle as usize)
            .copied()
            .unwrap_or(0)
    }
    /// 継ぎ目の縁の数（相手のあるアイランドの縁。両側を別に数える）。
    pub fn seam_edge_count(&self) -> usize {
        self.parts().seams.len()
    }

    /// 解像度 width × height のアイランドの図（覚えていればそれ）。作業予算は既定（[`DEFAULT_BUDGET`]）。
    pub fn island_map(&self, width: u32, height: u32) -> Result<Arc<IslandMap>, UvTopologyError> {
        self.island_map_within(width, height, DEFAULT_BUDGET)
    }

    /// 解像度 width × height のアイランドの図（覚えていればそれ。覚えているものは予算を問わず返す）。作るときは、作る途中の確保（行ごとの連なりと、
    /// それを 1 本にまとめる写し）が `budget` バイトに収まらなければ断る。
    pub fn island_map_within(
        &self,
        width: u32,
        height: u32,
        budget: u64,
    ) -> Result<Arc<IslandMap>, UvTopologyError> {
        if width == 0 || height == 0 || width > MAX_TOPOLOGY_EDGE || height > MAX_TOPOLOGY_EDGE {
            return Err(UvTopologyError::InvalidSize);
        }
        if let Some(m) = self.cached_map(width, height) {
            return Ok(m);
        }
        if self.refused_under(width, height, 0, budget) {
            return Err(UvTopologyError::Budget { budget });
        }
        let _gate = lock(&self.map_gate);
        if let Some(m) = self.cached_map(width, height) {
            return Ok(m); // ほかのスレッドが先に作った
        }
        let built = build_island_map(
            &self.geometry,
            self.material,
            self.parts(),
            width,
            height,
            budget,
            may_parallelize(),
        );
        let map = match built {
            Ok(map) => Arc::new(map),
            Err(e) => {
                if matches!(e, UvTopologyError::Budget { .. }) {
                    self.remember_refusal(width, height, 0, budget);
                }
                return Err(e);
            }
        };
        {
            let mut maps = lock(&self.maps);
            maps.push(map.clone());
            if maps.len() > KEEP {
                maps.remove(0);
            }
        }
        self.forget_refusal(width, height, 0);
        self.trim(budget);
        Ok(map)
    }

    /// 解像度 width × height、帯の幅 band（テクセル）の帯の写し（覚えていればそれ）。作業予算は既定（[`DEFAULT_BUDGET`]）。
    pub fn seam_band(
        &self,
        width: u32,
        height: u32,
        band: u32,
    ) -> Result<Arc<SeamBand>, UvTopologyError> {
        self.seam_band_within(width, height, band, DEFAULT_BUDGET)
    }

    /// 解像度 width × height、帯の幅 band（テクセル）の帯の写し（覚えていればそれ。覚えているものは予算を問わず返す）。作るときは、アイランドの図と
    /// 作る途中の確保（アイランドの中のビット・展開した三角形・行の帯ごとの作業・帯のテクセル）が `budget` バイトに収まらなければ断る。
    pub fn seam_band_within(
        &self,
        width: u32,
        height: u32,
        band: u32,
        budget: u64,
    ) -> Result<Arc<SeamBand>, UvTopologyError> {
        if band == 0
            || band > MAX_SEAM_BAND
            || width == 0
            || height == 0
            || width > MAX_TOPOLOGY_EDGE
            || height > MAX_TOPOLOGY_EDGE
        {
            return Err(UvTopologyError::InvalidSize);
        }
        if let Some(b) = self.cached_seam_band(width, height, band) {
            return Ok(b);
        }
        if self.refused_under(width, height, band, budget)
            || self.refused_under(width, height, 0, budget)
        {
            return Err(UvTopologyError::Budget { budget });
        }
        let _gate = lock(&self.band_gate);
        if let Some(b) = self.cached_seam_band(width, height, band) {
            return Ok(b); // ほかのスレッドが先に作った
        }
        let built = self
            .island_map_within(width, height, budget)
            .and_then(|islands| {
                super::seam_band::build(self, islands, band, budget, may_parallelize())
            });
        let built = match built {
            Ok(b) => Arc::new(b),
            Err(e) => {
                if matches!(e, UvTopologyError::Budget { .. }) {
                    self.remember_refusal(width, height, band, budget);
                }
                return Err(e);
            }
        };
        {
            let mut bands = lock(&self.bands);
            bands.push(built.clone());
            if bands.len() > KEEP {
                bands.remove(0);
            }
        }
        self.forget_refusal(width, height, band);
        self.trim(budget);
        Ok(built)
    }

    /// 覚えている帯の写し（作らない。無ければ None）。
    pub fn cached_seam_band(&self, width: u32, height: u32, band: u32) -> Option<Arc<SeamBand>> {
        lock(&self.bands)
            .iter()
            .find(|b| b.width() == width && b.height() == height && b.band() == band)
            .cloned()
    }

    /// 覚えているアイランドの図（作らない。無ければ None）。
    fn cached_map(&self, width: u32, height: u32) -> Option<Arc<IslandMap>> {
        lock(&self.maps)
            .iter()
            .find(|m| m.width == width && m.height == height)
            .cloned()
    }

    /// 覚えているアイランドの図と帯の写しのバイト数（名目。帯の写しが持つアイランドの図も、アイランドの図の覚えから外れていれば数える）。
    pub fn cached_bytes(&self) -> u64 {
        unique_bytes(&lock(&self.maps), &lock(&self.bands))
    }

    /// 解像度 width × height・帯の幅 band の帯の写しを、作業予算で断ったままか（作らずに調べるだけ。断ったことが無ければ、成功して
    /// 覚えているときも None）。アイランドの図を断ったときも、その大きさの帯の写しは断ったことになる。
    pub fn refusal(&self, width: u32, height: u32, band: u32) -> Option<UvTopologyError> {
        lock(&self.refused)
            .iter()
            .rev()
            .find(|r| r.width == width && r.height == height && (r.band == band || r.band == 0))
            .map(|r| UvTopologyError::Budget { budget: r.budget })
    }

    /// 同じ作り方を、この予算以下で断ったことがあるか。
    fn refused_under(&self, width: u32, height: u32, band: u32, budget: u64) -> bool {
        lock(&self.refused)
            .iter()
            .any(|r| r.width == width && r.height == height && r.band == band && budget <= r.budget)
    }

    fn remember_refusal(&self, width: u32, height: u32, band: u32, budget: u64) {
        let mut refused = lock(&self.refused);
        refused.retain(|r| !(r.width == width && r.height == height && r.band == band));
        refused.push(Refusal {
            width,
            height,
            band,
            budget,
        });
        if refused.len() > REFUSALS {
            refused.remove(0);
        }
    }

    fn forget_refusal(&self, width: u32, height: u32, band: u32) {
        lock(&self.refused).retain(|r| !(r.width == width && r.height == height && r.band == band));
    }

    /// 覚えているアイランドの図と帯の写しの合計を、予算に収まるまで古いものから捨てる（直近の 1 つずつは残す）。
    fn trim(&self, budget: u64) {
        self.evict(budget, 1);
    }

    /// 覚えているアイランドの図と帯の写しの合計が `budget` を超えていれば、収まるまで古いものから捨てる（直近のものも。予算を小さくしたとき）。
    pub fn shrink(&self, budget: u64) {
        self.evict(budget, 0);
    }

    fn evict(&self, budget: u64, keep: usize) {
        let mut maps = lock(&self.maps);
        let mut bands = lock(&self.bands);
        while unique_bytes(&maps, &bands) > budget {
            if bands.len() > keep {
                bands.remove(0);
            } else if maps.len() > keep {
                maps.remove(0);
            } else {
                break;
            }
        }
    }
}

/// アイランドの図と帯の写しの合計のバイト数（帯の写しが持つアイランドの図は、覚えにあるものと同じなら 1 度だけ数える）。
fn unique_bytes(maps: &[Arc<IslandMap>], bands: &[Arc<SeamBand>]) -> u64 {
    let mut seen: Vec<*const IslandMap> = maps.iter().map(Arc::as_ptr).collect();
    let mut total: u64 = maps.iter().map(|m| m.bytes()).sum();
    for b in bands {
        total += b.bytes();
        let islands = b.islands();
        if !seen.contains(&Arc::as_ptr(islands)) {
            seen.push(Arc::as_ptr(islands));
            total += islands.bytes();
        }
    }
    total
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// 今のスレッドから、位相の対応・アイランドの図・帯の写しを並列（rayon）で作ってよいか。rayon のスレッドの中（評価のブロックを並べている最中など）では
/// 作らない: 作る間は門・`OnceLock` を持っていて、並列の仕事の終わりを待つスレッドは、同じスレッドの中で別の仕事を拾う。拾った仕事が同じ門・`OnceLock` を
/// 取りに来ると、自分が持つものを待って止まる。rayon の外（アプリの主のスレッドや裏の仕事）からは、今までどおり並列にする。結果は並列でも逐次でも同じ。
pub(crate) fn may_parallelize() -> bool {
    rayon::current_thread_index().is_none()
}

/// `0..n` の各番号に `f` を当てた結果を、番号の順に集める。`parallel` なら rayon で、そうでなければ今のスレッドで順に（結果は同じ）。
pub(crate) fn collect_indexed<T, R>(
    parallel: bool,
    n: usize,
    min_len: usize,
    f: impl Fn(usize) -> T + Sync + Send,
) -> R
where
    T: Send,
    R: FromIterator<T> + FromParallelIterator<T>,
{
    if parallel {
        (0..n)
            .into_par_iter()
            .with_min_len(min_len.max(1))
            .map(f)
            .collect()
    } else {
        (0..n).map(f).collect()
    }
}

/// 2 つの UV が同じか（`project` の継ぎ目の見つけ方と同じ許し）。
pub(crate) fn same_uv(a: Vec2, b: Vec2) -> bool {
    (a.x - b.x).abs() <= 1e-6 && (a.y - b.y).abs() <= 1e-6
}

impl Parts {
    /// `parallel` なら辺の向こうの検索を rayon で並べる（結果は逐次と同じ）。
    pub(crate) fn new(g: &SurfaceGeometry, material: Option<i32>, parallel: bool) -> Parts {
        let tris = g.triangles();
        let n = tris.len();
        let member = |i: usize| material.is_none_or(|m| tris[i].material == m);
        // アイランド: UV の辺を共有する（同じスロットの）三角形をつなぐ
        let key = |uv: Vec2| {
            (
                (uv.x as f64 * 1e6).round_ties_even() as i64,
                (uv.y as f64 * 1e6).round_ties_even() as i64,
            )
        };
        let mut parent: Vec<u32> = (0..n as u32).collect();
        fn find(parent: &mut [u32], mut i: u32) -> u32 {
            while parent[i as usize] != i {
                let up = parent[parent[i as usize] as usize];
                parent[i as usize] = up;
                i = up;
            }
            i
        }
        type UvKey = (i64, i64);
        let mut first: FastMap<(i32, UvKey, UvKey), u32> = FastMap::default();
        for (i, t) in tris.iter().enumerate() {
            if !member(i) {
                continue;
            }
            let k = [key(t.uv_a), key(t.uv_b), key(t.uv_c)];
            for e in 0..3 {
                let (a, b) = (k[e], k[(e + 1) % 3]);
                let edge = if a <= b {
                    (t.material_slot, a, b)
                } else {
                    (t.material_slot, b, a)
                };
                match first.entry(edge) {
                    Entry::Occupied(o) => {
                        let (x, y) = (find(&mut parent, i as u32), find(&mut parent, *o.get()));
                        if x != y {
                            // 小さい番号を根にする（アイランドの番号を一番小さい三角形の順にしやすく）
                            let (lo, hi) = (x.min(y), x.max(y));
                            parent[hi as usize] = lo;
                        }
                    }
                    Entry::Vacant(v) => {
                        v.insert(i as u32);
                    }
                }
            }
        }
        let mut island_of = vec![0u32; n];
        let mut numbers: FastMap<u32, u32> = FastMap::default();
        let mut count = 0u32;
        for (i, slot) in island_of.iter_mut().enumerate() {
            if !member(i) {
                continue;
            }
            let root = find(&mut parent, i as u32);
            *slot = *numbers.entry(root).or_insert_with(|| {
                count += 1;
                count
            });
        }
        // 辺の向こうと継ぎ目の縁
        let tolerance = g.weld_tolerance() as f64;
        let links: Vec<[Link; 3]> = collect_indexed(parallel, n, 1024, |i| {
            if member(i) {
                links_of(g, i, tolerance, &member)
            } else {
                [Link::NONE; 3]
            }
        });
        // アイランドの縁（継ぎ目と相手の無い辺）を、始まり・終わりの UV でつなぐ（向きの揃ったアイランドでは、縁の頂点ごとに出る縁と入る縁が 1 つずつ）
        let uv_of = |i: usize, j: usize| {
            let t = &tris[i];
            [t.uv_a, t.uv_b, t.uv_c][j % 3]
        };
        let mut starts: FastMap<(u32, UvKey), Vec<(u32, u8)>> = FastMap::default();
        let mut ends: FastMap<(u32, UvKey), Vec<(u32, u8)>> = FastMap::default();
        for (i, l) in links.iter().enumerate() {
            if island_of[i] == 0 {
                continue;
            }
            for (j, link) in l.iter().enumerate() {
                if !link.continuous {
                    starts
                        .entry((island_of[i], key(uv_of(i, j))))
                        .or_default()
                        .push((i as u32, j as u8));
                    ends.entry((island_of[i], key(uv_of(i, j + 1))))
                        .or_default()
                        .push((i as u32, j as u8));
                }
            }
        }
        let one = |list: Option<&Vec<(u32, u8)>>| match list.map(Vec::as_slice) {
            Some([only]) => Some(*only),
            _ => None,
        };
        let mut seams = Vec::new();
        for (i, l) in links.iter().enumerate() {
            for (edge, link) in l.iter().enumerate() {
                if link.neighbor != u32::MAX && !link.continuous {
                    let island = island_of[i];
                    let next = one(starts.get(&(island, key(uv_of(i, edge + 1)))))
                        .map(|(t, j)| uv_of(t as usize, j as usize + 1));
                    let prev = one(ends.get(&(island, key(uv_of(i, edge)))))
                        .map(|(t, j)| uv_of(t as usize, j as usize));
                    seams.push(SeamEdge {
                        triangle: i as u32,
                        edge: edge as u8,
                        partner: link.neighbor,
                        partner_a: link.a,
                        partner_b: link.b,
                        prev,
                        next,
                    });
                }
            }
        }
        Parts {
            island_of,
            island_count: count,
            seams,
            links,
        }
    }
}

/// 三角形 i の辺ごとの向こう。隣の三角形が同じ 2 頂点（溶接の鍵）で UV も同じならアイランドの中の辺、同じ 2 頂点で UV が違えば継ぎ目。
fn links_of(
    g: &SurfaceGeometry,
    i: usize,
    tolerance: f64,
    member: &(dyn Fn(usize) -> bool + Sync),
) -> [Link; 3] {
    let tris = g.triangles();
    let t = &tris[i];
    let keys = [
        position_key(t.a, tolerance),
        position_key(t.b, tolerance),
        position_key(t.c, tolerance),
    ];
    let uvs = [t.uv_a, t.uv_b, t.uv_c];
    let mut out = [Link::NONE; 3];
    for (edge, slot) in out.iter_mut().enumerate() {
        let (a, b) = (edge, (edge + 1) % 3);
        let mut partner = None;
        let mut continues = false;
        for &n in g.neighbors(i) {
            if !member(n as usize) {
                continue;
            }
            let o = &tris[n as usize];
            let okeys = [
                position_key(o.a, tolerance),
                position_key(o.b, tolerance),
                position_key(o.c, tolerance),
            ];
            let ouvs = [o.uv_a, o.uv_b, o.uv_c];
            let ia = okeys.iter().position(|k| *k == keys[a]);
            let ib = okeys.iter().position(|k| *k == keys[b]);
            if let (Some(ia), Some(ib)) = (ia, ib) {
                if ia == ib {
                    continue;
                }
                if same_uv(ouvs[ia], uvs[a]) && same_uv(ouvs[ib], uvs[b]) {
                    *slot = Link {
                        neighbor: n,
                        a: ia as u8,
                        b: ib as u8,
                        continuous: true,
                    };
                    continues = true;
                    break;
                }
                if partner.is_none() {
                    partner = Some((n, ia as u8, ib as u8));
                }
            }
        }
        if let (false, Some((n, ia, ib))) = (continues, partner) {
            *slot = Link {
                neighbor: n,
                a: ia,
                b: ib,
                continuous: false,
            };
        }
    }
    out
}

/// アイランドの図を作る（ベイクの行の割り当てを 1 テクセル 1 点で）。作る途中の確保（行ごとの連なり、それを 1 本にまとめる写しとの 2 つ分と、
/// 行ごとの入れ物）が `budget` に収まらなければ、行の連なりを数えながら途中で断る。
pub(crate) fn build_island_map(
    g: &SurfaceGeometry,
    material: Option<i32>,
    parts: &Parts,
    width: u32,
    height: u32,
    budget: u64,
    parallel: bool,
) -> Result<IslandMap, UvTopologyError> {
    let over = UvTopologyError::Budget { budget };
    let run_bytes = std::mem::size_of::<IslandRun>() as u64;
    // 行ごとの入れ物（`Vec<IslandRun>` の本体）と、行の始まりの表
    let rows_bytes = u64::from(height) * (std::mem::size_of::<Vec<IslandRun>>() as u64 + 4) + 4;
    if rows_bytes > budget {
        return Err(over);
    }
    let tris = g.triangles();
    let mut uvs = Vec::with_capacity(tris.len() * 6);
    for t in tris {
        uvs.extend_from_slice(&[t.uv_a.x, t.uv_a.y, t.uv_b.x, t.uv_b.y, t.uv_c.x, t.uv_c.y]);
    }
    // ベイクと同じく、UV の面積の無い三角形は割り当てない
    let receivers: Vec<usize> = (0..tris.len())
        .filter(|&i| material.is_none_or(|m| tris[i].material == m))
        .filter(|&i| {
            let u = &uvs[i * 6..i * 6 + 6];
            let area = ((u[2] - u[0]) * (u[5] - u[1]) - (u[4] - u[0]) * (u[3] - u[1])) as f64;
            area.abs() * width as f64 * (height as f64) >= 1e-9
        })
        .collect();
    // 行の割り当ては、行の帯への参照のバイト数が budget を超えるときだけ断る（これ以外の理由では断らない）
    let raster = Raster::from_uvs(&uvs, (width as i32, height as i32), 1, &receivers, budget)
        .map_err(|_| over.clone())?;
    let w = width as usize;
    // 連なりのバイト数（行ごとの入れ物に入っている分）。まとめるとき同じだけ増えるので、2 倍まで
    let held = std::sync::atomic::AtomicU64::new(0);
    let limit = (budget - rows_bytes) / 2;
    let rows: Result<Vec<Vec<IslandRun>>, UvTopologyError> =
        collect_indexed(parallel, height as usize, 1, |y| {
            let samples = raster.row(y, w, 1);
            let mut runs: Vec<IslandRun> = Vec::new();
            for x in 0..w {
                let Some(s) = samples.get(x) else {
                    continue;
                };
                let island = parts.island_of[s.triangle];
                let x = x as u32;
                match runs.last_mut() {
                    Some(r) if r.end == x && r.island == island && r.overlap == s.overlap => {
                        r.end += 1
                    }
                    _ => runs.push(IslandRun {
                        start: x,
                        end: x + 1,
                        island,
                        overlap: s.overlap,
                    }),
                }
            }
            let bytes = runs.capacity() as u64 * run_bytes;
            if held.fetch_add(bytes, std::sync::atomic::Ordering::Relaxed) + bytes > limit {
                return Err(UvTopologyError::Budget { budget });
            }
            Ok(runs)
        });
    let rows = rows?;
    let mut offsets = Vec::with_capacity(rows.len() + 1);
    offsets.push(0u32);
    let total: usize = rows.iter().map(Vec::len).sum();
    let mut runs = Vec::with_capacity(total);
    for r in rows {
        runs.extend(r);
        offsets.push(runs.len() as u32);
    }
    Ok(IslandMap {
        width,
        height,
        island_count: parts.island_count,
        rows: offsets,
        runs,
    })
}
