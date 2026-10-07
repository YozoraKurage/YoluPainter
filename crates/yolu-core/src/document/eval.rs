//! 効果の評価と、評価した出力のキャッシュ（C# の `FilterEngine` の文書側: 設定・入力・元画素から、層の出力を合成が読む形で作る）。
//!
//! - 評価は文書に依らない口（[`crate::filter`]・[`crate::generator`]・[`crate::fill_image`]）を呼ぶだけで、式をここに持たない。
//! - 作業の単位は、タイルの大きさに揃えたブロック（既定 256 画素四方。結果はブロックの大きさによらない）。評価した出力は
//!   「評価済みの面」（[`Surface`]）のタイルとして合成に渡し、合成の式は元の画素の面を読むときと同じ。
//! - キャッシュは派生の表示用（保存の正本にしない）。鍵（[`Stamp`]）は、層の効果の設定そのものの写し・元画素のタイルごとの通し番号
//!   （ぼかしなどの半径の分だけ広げた窓。正規化は全部）・外から渡した入力の版・Anchor を読む段では変化の記録の通し番号。
//!   鍵が等しければ出力は等しい。予算を超えたら古いブロックから捨てる。
//! - 評価の途中で取り消したら、完成したブロックだけがキャッシュに残る（途中の画素は公開しない）。

use std::collections::HashMap;
use std::sync::atomic::AtomicBool;

use rayon::prelude::*;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use super::Document;
use crate::composite::{Entry, Stack};
use crate::effects::{
    value_type_of, AnchorPlacement, EffectInputs, EffectSettings, FilterEffect, ImageId,
    InactiveReason,
};
use crate::error::CoreError;
use crate::fill_image::{
    self, Conversion, FillInput, FillSampler, ImageMipChain, Projection, ProjectionMode,
};
use crate::filter::{self, GeneratorInput, ValueType};
use crate::generator::{self, anchor, BoundGenerator, MapKind, MapState};
use crate::layer::LayerId;
use crate::surface::{Pixels, Surface, Tile};
use crate::types::{Channel, ChannelKind, LayerKind, Rect, Rgba8, TileCoord};

/// 評価の入力の元: 層のチャンネルの画素（内容）か、層のラスターマスク。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) enum SourceKey {
    Channel(Channel),
    Mask,
}

/// タイルの範囲（終わりを含まない）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct TileRange {
    pub x0: u32,
    pub y0: u32,
    pub x1: u32,
    pub y1: u32,
}

impl TileRange {
    fn iter(self) -> impl Iterator<Item = TileCoord> {
        (self.y0..self.y1).flat_map(move |y| (self.x0..self.x1).map(move |x| TileCoord::new(x, y)))
    }
    fn contains(self, c: TileCoord) -> bool {
        c.x >= self.x0 && c.x < self.x1 && c.y >= self.y0 && c.y < self.y1
    }
}

/// 評価の出力を作るタイルの範囲: 長方形か、散らばったタイルの組（合成のタイルの束。範囲の外のブロックは評価しない）。
pub(crate) enum Region<'r> {
    Range(TileRange),
    Tiles(&'r [TileCoord]),
}

/// 歩幅つきの読み元（粗い評価）: 粗い画素 (x, y) は元の画素 (x·歩幅, y·歩幅)。
struct StridedSource<'e> {
    inner: &'e dyn filter::Source,
    stride: u32,
    coarse: (u32, u32),
    full: (u32, u32),
}

impl filter::Source for StridedSource<'_> {
    fn dimensions(&self) -> (u32, u32) {
        self.coarse
    }
    fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        self.inner.pixel(
            (x * self.stride).min(self.full.0 - 1),
            (y * self.stride).min(self.full.1 - 1),
        )
    }
}

/// 粗い評価の Generator: 粗い画素 (x, y) の値は、元の画素 (x·歩幅, y·歩幅) での値。
/// 行の評価（`sample_row`）は置き換えず、1 画素ずつ読む: 粗い行の画素は元の行で歩幅ごとに離れていて、元の行を続けて作ってから拾うと
/// 歩幅の倍の画素を作る（4096²・1 スレッドの `generator_stage_bench` で、歩幅 4 は 3 段の層で 1 画素ずつと同じ程度・ランプの段で 1.4〜2.5 倍、
/// 歩幅 8・16 は 1.6〜6.2 倍遅かった）。
struct ScaledGenerators<'e> {
    inner: &'e dyn GeneratorInput,
    stride: u32,
    full: (u32, u32),
}

impl GeneratorInput for ScaledGenerators<'_> {
    fn sample(&self, slot: u32, x: u32, y: u32) -> Option<filter::Generated> {
        self.inner.sample(
            slot,
            (x * self.stride).min(self.full.0 - 1),
            (y * self.stride).min(self.full.1 - 1),
        )
    }
}

/// 粗い評価の段: ぼかし・シャープの半径を歩幅で割る（丸めて 0 になる段は外す。半径が歩幅の半分に満たないぼかしは、粗い絵では見えない）。
/// ほかの段は点ごとの処理かノイズ・正規化なので、そのまま。
fn coarse_stages(stages: &[filter::Stage], stride: u32) -> Vec<filter::Stage> {
    let reduce = |r: u32| (r + stride / 2) / stride;
    stages
        .iter()
        .map(|stage| {
            let mut stage = stage.clone();
            let keep = match &mut stage.settings {
                filter::Settings::GaussianBlur { radius }
                | filter::Settings::Sharpen { radius, .. } => {
                    let r = reduce(*radius);
                    *radius = r.max(1);
                    r > 0
                }
                s if s.is_spatial() => match s.coarse(stride) {
                    Some(c) => {
                        *s = c;
                        true
                    }
                    None => false,
                },
                _ => true,
            };
            stage.enabled &= keep;
            stage
        })
        .collect()
}

/// 評価の仕様の段を、作業メモリの見積りに使うフィルターの段の並びへ（強さと有効は仕様のまま）。
pub(super) fn spec_stages(spec: &Spec) -> Vec<filter::Stage> {
    spec.config
        .chain
        .iter()
        .map(|e| filter::Stage {
            settings: match &e.settings {
                EffectSettings::Filter(f) => f.clone(),
                EffectSettings::Generator(g) => filter::Settings::Generator {
                    slot: 0,
                    blend: super::effects::generator_blend(g.blend),
                },
            },
            enabled: true,
            strength: e.strength,
        })
        .collect()
}

/// 評価の状態（文書の一部。保存も Undo もしない）。
pub(crate) struct EffectState {
    pub inputs: EffectInputs,
    /// マップ・モデルのルート・画像（位相以外の入力）が替わるたびに増える（それを読む段の評価の鍵）。
    pub inputs_revision: u64,
    /// モデルの UV の位相が別の物に替わるたびに増える（継ぎ目をまたぐ段の評価の鍵。マップや画像の差し替えでは増えない）。
    pub topology_revision: u64,
    /// 読み込み・直接の書き込みのたびに増える（キャッシュを全部無効にする）。
    pub generation: u64,
    /// 元画素の変化の時計（タイルが変わるたびに進む）。
    pub clock: u64,
    /// 層の元画素のタイルごとの、最後に変わったときの時計。
    pub source: HashMap<(LayerId, SourceKey), HashMap<TileCoord, u64>>,
    /// マスクの Anchor を持つ層ごとに、マスクが最後に変わった変化の記録の通し番号。
    pub mask_serials: HashMap<LayerId, u64>,
    /// Anchor を読む段の解決の署名（変わったら、読む層を全部変わったことにする）。
    pub anchor_signature: Vec<AnchorSig>,
    pub cache: Mutex<EvalCache>,
    pub working_budget: u64,
    pub cache_budget: u64,
    pub image_cache_budget: u64,
    /// モデルの UV の位相が覚える島の図・帯の写しと、それを作る間の作業メモリの予算（`uv_seams`）。
    pub seam_budget: u64,
    pub block_pixels: u32,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct AnchorSig {
    pub reader: LayerId,
    pub filter: crate::effects::FilterId,
    pub resolved: Result<(LayerId, AnchorPlacement), anchor::Issue>,
    pub channel: Channel,
    pub read: anchor::ReadMode,
}

impl Default for EffectState {
    fn default() -> Self {
        EffectState {
            inputs: EffectInputs::default(),
            inputs_revision: 1,
            topology_revision: 1,
            generation: 1,
            clock: 0,
            source: HashMap::new(),
            mask_serials: HashMap::new(),
            anchor_signature: Vec::new(),
            cache: Mutex::new(EvalCache::default()),
            working_budget: 256 * 1024 * 1024,
            cache_budget: 256 * 1024 * 1024,
            image_cache_budget: 256 * 1024 * 1024,
            seam_budget: crate::geometry::DEFAULT_BUDGET,
            block_pixels: 256,
        }
    }
}

/// 評価の回数（診断・試験）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EffectCounters {
    /// 評価したブロックの数。
    pub blocks_evaluated: u64,
    /// 合成した Anchor のタイルの数。
    pub anchor_tiles_composited: u64,
    /// 作ったミップマップの数。
    pub mip_chains_built: u64,
    /// 正規化の統計を求めた回数。
    pub statistics_computed: u64,
    /// 持っている評価済みのタイルのバイト数。
    pub cache_bytes: u64,
    /// 持っているミップマップのバイト数。
    pub image_cache_bytes: u64,
}

#[derive(Default)]
pub(crate) struct EvalCache {
    blocks: HashMap<BlockKey, BlockEntry>,
    stats: HashMap<(LayerId, SourceKey), StatsEntry>,
    anchors: HashMap<(LayerId, Channel, TileCoord), AnchorEntry>,
    chains: HashMap<(String, u8, bool), ChainEntry>,
    bytes: u64,
    anchor_bytes: u64,
    chain_bytes: u64,
    clock: u64,
    counters: EffectCounters,
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
struct BlockKey {
    layer: LayerId,
    key: SourceKey,
    bx: u32,
    by: u32,
}

type BlockTiles = Arc<Vec<(TileCoord, Option<Tile>)>>;

struct BlockEntry {
    stamp: Stamp,
    tiles: BlockTiles,
    bytes: u64,
    used: u64,
}
struct StatsEntry {
    stamp: Stamp,
    stats: Arc<Vec<Option<filter::Statistics>>>,
}
struct AnchorEntry {
    stamp: (u64, u64),
    tile: Option<Arc<Vec<u8>>>,
    used: u64,
}
struct ChainEntry {
    chain: Arc<ImageMipChain<'static>>,
    used: u64,
}

/// 出力が等しいことの鍵。
#[derive(Clone, PartialEq, Debug)]
struct Stamp {
    config: Arc<Config>,
    generation: u64,
    source: u64,
    inputs: u64,
    /// 継ぎ目をまたぐ段が読むモデルの UV の位相の世代（またがない段は 0）。
    topology: u64,
    anchors: u64,
}

/// 出力が依る設定（等しければ出力の式が同じ）。
#[derive(Clone, PartialEq, Debug)]
struct Config {
    /// 有効な段（スタックの順）。
    chain: Vec<FilterEffect>,
    value_type: ValueType,
    fill: Option<FillConfig>,
    /// 段ごとの Anchor の解決（Anchor の Generator でない段は None）。
    anchors: Vec<Option<AnchorStage>>,
    /// UV の継ぎ目をまたぐ帯の幅（0 はまたがない。`uv_seams`）。
    seam_band: u32,
}

#[derive(Clone, PartialEq, Debug)]
struct FillConfig {
    value: Rgba8,
    kind: ChannelKind,
    image: Option<ImageId>,
    gradient: Option<generator::Settings>,
    projection: Projection,
    /// デカールの形に使う別チャンネルの画像（チャンネルの順で最初の画像のチャンネル。自分なら None）。
    shape: Option<(Channel, ImageId)>,
    decal: bool,
}

#[derive(Clone, PartialEq, Debug)]
enum AnchorStage {
    Bound(AnchorRef),
    Unusable(anchor::Issue),
}

#[derive(Clone, PartialEq, Debug)]
struct AnchorRef {
    host: LayerId,
    placement: AnchorPlacement,
    channel: Channel,
    read: anchor::ReadMode,
}

/// 層・チャンネル（またはマスク）の評価の指定。
pub(super) struct Spec {
    layer: usize,
    id: LayerId,
    key: SourceKey,
    config: Arc<Config>,
    halo: u32,
    global: bool,
    reads_inputs: bool,
    reads_anchor: bool,
    /// UV の継ぎ目をまたぐ帯の幅と近傍の段の数（帯の幅 0 はまたがない。`uv_seams`）。
    pub(super) seam: (u32, usize),
}

/// 合成が読む評価済みの面（層の番号から）。内容とマスクは別。
#[derive(Default)]
pub(crate) struct EvalSet {
    pub content: HashMap<usize, Surface>,
    pub masks: HashMap<usize, Surface>,
    /// 面が粗く評価したもの（歩幅。幅・高さは文書の 1/歩幅、タイルの一辺は文書のタイルの 1/歩幅）なら、その歩幅。
    pub reduced: Option<u32>,
}

impl EvalSet {
    /// 層 `from` の出力だけを、番号 `to` で持つ写し（面のタイルは共有する）。
    pub(crate) fn only(&self, from: usize, to: usize) -> EvalSet {
        let pick = |m: &HashMap<usize, Surface>| m.get(&from).map(|s| (to, s.clone()));
        EvalSet {
            content: pick(&self.content).into_iter().collect(),
            masks: pick(&self.masks).into_iter().collect(),
            reduced: self.reduced,
        }
    }
}

pub(crate) fn map_filter_error(e: filter::Error) -> CoreError {
    match e {
        filter::Error::Invalid(why) => CoreError::InvalidArgument(why),
        filter::Error::Budget { .. } | filter::Error::Allocation => {
            CoreError::WorkingBudgetExceeded
        }
        filter::Error::Cancelled => CoreError::Cancelled,
    }
}
fn map_generator_error(e: generator::Error) -> CoreError {
    match e {
        generator::Error::Invalid(why) => CoreError::InvalidArgument(why),
        generator::Error::Budget { .. } | generator::Error::Allocation => {
            CoreError::WorkingBudgetExceeded
        }
        generator::Error::Cancelled => CoreError::Cancelled,
    }
}
fn map_fill_error(e: fill_image::FillError) -> CoreError {
    match e {
        fill_image::FillError::Invalid(_) => CoreError::InvalidArgument("塗りつぶしの入力"),
        fill_image::FillError::OverBudget { .. } | fill_image::FillError::Allocation => {
            CoreError::WorkingBudgetExceeded
        }
        fill_image::FillError::Canceled => CoreError::Cancelled,
    }
}
pub(super) fn cancelled(cancel: Option<&AtomicBool>) -> Result<(), CoreError> {
    if cancel.is_some_and(|c| c.load(std::sync::atomic::Ordering::Relaxed)) {
        Err(CoreError::Cancelled)
    } else {
        Ok(())
    }
}

/// 評価の途中の読み元: タイルの面（層の画素・マスク）。ディスクから読めないタイルは誤りを覚えて 0 を返す（評価の後に
/// `with_env` が誤りを返すので、その出力は使わない）。
struct SurfaceSource<'a> {
    surface: Option<&'a Surface>,
    width: u32,
    height: u32,
    failed: OnceLock<CoreError>,
}

impl SurfaceSource<'_> {
    fn latch(&self, e: CoreError) {
        let _ = self.failed.set(e);
    }
}

impl filter::Source for SurfaceSource<'_> {
    fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }
    fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        match self.surface.map(|s| s.pixel(x, y)) {
            None | Some(Err(CoreError::InvalidArgument(_))) => [0; 4],
            Some(Ok(p)) => p.to_array(),
            Some(Err(e)) => {
                self.latch(e);
                [0; 4]
            }
        }
    }
    fn read_row(&self, x: u32, y: u32, out: &mut [u8]) {
        let Some(surface) = self.surface else {
            out.fill(0);
            return;
        };
        if let Err(e) = surface.read_row(x, y, out) {
            out.fill(0);
            self.latch(e);
        }
    }
}

/// 塗りつぶしの層のチャンネルの画素（値・グラデーション・投影した画像）。
struct FillSource<'a> {
    value: Rgba8,
    width: u32,
    height: u32,
    gradient: Option<BoundGenerator<'a>>,
    scalar: bool,
    sampler: Option<FillSampler<'a>>,
    decal: bool,
}

impl filter::Source for FillSource<'_> {
    fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }
    fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        if let Some(g) = &self.gradient {
            // 置き換えのランプ付きのグラデーション: マップが使えない所は値のまま
            let mut v = self.value;
            if let Some(generator::Generated::Mapped(p)) = g.sample(x, y, self.scalar) {
                v = Rgba8::from_slice(&p);
            }
            if self.decal {
                if let Some(s) = &self.sampler {
                    v = s
                        .apply_decal_to_value(x, y, v)
                        .unwrap_or(Rgba8::TRANSPARENT);
                }
            }
            return v.to_array();
        }
        if let Some(s) = &self.sampler {
            return s.pixel(x, y).unwrap_or(self.value).to_array();
        }
        self.value.to_array()
    }
}

enum ChainSource<'a> {
    Surface(SurfaceSource<'a>),
    Fill(Box<FillSource<'a>>),
}

impl filter::Source for ChainSource<'_> {
    fn dimensions(&self) -> (u32, u32) {
        match self {
            Self::Surface(s) => s.dimensions(),
            Self::Fill(s) => s.dimensions(),
        }
    }
    fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        match self {
            Self::Surface(s) => s.pixel(x, y),
            Self::Fill(s) => s.pixel(x, y),
        }
    }
    fn read_row(&self, x: u32, y: u32, out: &mut [u8]) {
        match self {
            Self::Surface(s) => s.read_row(x, y, out),
            Self::Fill(s) => {
                for (i, p) in out.chunks_exact_mut(4).enumerate() {
                    p.copy_from_slice(&s.pixel(x + i as u32, y));
                }
            }
        }
    }
}

/// 段ごとの Generator（束縛済み）。フィルターの評価器の `GeneratorInput` として、段の番号で引く。
struct StageInput<'e> {
    bound: Vec<Option<BoundGenerator<'e>>>,
    scalar: bool,
}

impl GeneratorInput for StageInput<'_> {
    fn sample(&self, slot: u32, x: u32, y: u32) -> Option<filter::Generated> {
        let b = self.bound.get(slot as usize)?.as_ref()?;
        b.sample(x, y, self.scalar)
    }
    fn sample_row(&self, slot: u32, x0: u32, y: u32, out: &mut [Option<filter::Generated>]) {
        match self.bound.get(slot as usize).and_then(Option::as_ref) {
            Some(b) => b.sample_row(x0, y, self.scalar, out),
            None => out.fill(None),
        }
    }
}

/// Anchor が読むタイルの並び（長方形の範囲。無いタイルは透明）。評価器の `Source`（画素は straight RGBA8）として読ませる。
struct TileGrid {
    tiles: Vec<Option<Arc<Vec<u8>>>>,
    range: TileRange,
    tile_size: u32,
    dims: (u32, u32),
}

impl generator::Source for TileGrid {
    fn dimensions(&self) -> (u32, u32) {
        self.dims
    }
    fn pixel(&self, x: u32, y: u32) -> Rgba8 {
        let ts = self.tile_size;
        let (tx, ty) = (x / ts, y / ts);
        if tx < self.range.x0 || tx >= self.range.x1 || ty < self.range.y0 || ty >= self.range.y1 {
            return Rgba8::TRANSPARENT;
        }
        let cols = (self.range.x1 - self.range.x0) as usize;
        let slot = (ty - self.range.y0) as usize * cols + (tx - self.range.x0) as usize;
        match &self.tiles[slot] {
            None => Rgba8::TRANSPARENT,
            Some(t) => {
                let at = (((y % ts) * ts + x % ts) * 4) as usize;
                Rgba8::from_slice(&t[at..at + 4])
            }
        }
    }
}

/// 1 枚のタイルだけを読ませる（Anchor の合成の入力）。ほかの画素は透明。
struct TileView {
    coord: TileCoord,
    tile_size: u32,
    dims: (u32, u32),
    tile: Option<Pixels>,
}

impl generator::Source for TileView {
    fn dimensions(&self) -> (u32, u32) {
        self.dims
    }
    fn pixel(&self, x: u32, y: u32) -> Rgba8 {
        let ts = self.tile_size;
        if x / ts != self.coord.x || y / ts != self.coord.y {
            return Rgba8::TRANSPARENT;
        }
        match &self.tile {
            None => Rgba8::TRANSPARENT,
            Some(t) => t.get((((y % ts) * ts + x % ts) * 4) as usize),
        }
    }
}

enum AnchorValue<'a> {
    Layer(anchor::LayerSample<'a>),
    Mask(anchor::MaskSample<'a>),
}

impl anchor::ValueSource for AnchorValue<'_> {
    fn dimensions(&self) -> (u32, u32) {
        match self {
            Self::Layer(s) => s.dimensions(),
            Self::Mask(s) => s.dimensions(),
        }
    }
    fn value(&self, x: u32, y: u32) -> Option<f64> {
        match self {
            Self::Layer(s) => s.value(x, y),
            Self::Mask(s) => s.value(x, y),
        }
    }
}

/// 評価の環境（読み元・段・Generator）。`with_env` の中でだけ生きる。
struct Env<'e> {
    source: &'e dyn filter::Source,
    value_type: ValueType,
    stages: &'e [filter::Stage],
    generators: &'e dyn GeneratorInput,
}

impl Document {
    // ───────── 状態・診断 ─────────

    fn cache(&self) -> MutexGuard<'_, EvalCache> {
        self.effects
            .cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// 評価の回数・キャッシュの大きさ（診断・試験）。
    pub fn effect_counters(&self) -> EffectCounters {
        let c = self.cache();
        EffectCounters {
            cache_bytes: c.bytes + c.anchor_bytes,
            image_cache_bytes: c.chain_bytes,
            ..c.counters
        }
    }

    /// 持っている評価済みのタイルを全部捨てる（次に読むときに作り直す）。
    pub fn release_effect_cache(&self) {
        let mut c = self.cache();
        c.blocks.clear();
        c.stats.clear();
        c.anchors.clear();
        c.chains.clear();
        c.bytes = 0;
        c.anchor_bytes = 0;
        c.chain_bytes = 0;
    }

    pub(super) fn trim_effect_cache(&self) {
        let budget = self.effects.cache_budget;
        let image_budget = self.effects.image_cache_budget;
        let mut c = self.cache();
        if c.bytes > budget {
            let mut all: Vec<(u64, BlockKey, u64)> = c
                .blocks
                .iter()
                .map(|(k, e)| (e.used, k.clone(), e.bytes))
                .collect();
            all.sort_by_key(|e| e.0);
            for (_, key, bytes) in all {
                if c.bytes <= budget / 4 * 3 {
                    break;
                }
                c.blocks.remove(&key);
                c.bytes -= bytes;
            }
        }
        if c.anchor_bytes > budget {
            let mut all: Vec<(u64, (LayerId, Channel, TileCoord), u64)> = c
                .anchors
                .iter()
                .map(|(k, e)| (e.used, *k, e.tile.as_ref().map_or(0, |t| t.len() as u64)))
                .collect();
            all.sort_by_key(|e| e.0);
            for (_, key, bytes) in all {
                if c.anchor_bytes <= budget / 4 * 3 {
                    break;
                }
                c.anchors.remove(&key);
                c.anchor_bytes -= bytes;
            }
        }
        if c.chain_bytes > image_budget {
            let mut all: Vec<(u64, (String, u8, bool), u64)> = c
                .chains
                .iter()
                .map(|(k, e)| (e.used, k.clone(), e.chain.bytes()))
                .collect();
            all.sort_by_key(|e| e.0);
            for (_, key, bytes) in all {
                if c.chain_bytes <= image_budget {
                    break;
                }
                c.chains.remove(&key);
                c.chain_bytes -= bytes;
            }
        }
    }

    // ───────── 元画素の変化の記録 ─────────

    /// 層の元画素（チャンネルの面かマスク）のタイルが変わったことを覚える（評価のキャッシュの鍵）。
    pub(super) fn note_source(&mut self, index: usize, target: super::Target, coord: TileCoord) {
        let id = self.layers[index].id;
        let key = match target {
            super::Target::Channel(c) => SourceKey::Channel(c),
            super::Target::Mask => SourceKey::Mask,
        };
        self.effects.clock += 1;
        let serial = self.effects.clock;
        self.effects
            .source
            .entry((id, key))
            .or_default()
            .insert(coord, serial);
        if matches!(target, super::Target::Mask) {
            self.note_mask_output(index);
        }
    }

    /// 層のマスクの出力が変わり得ることを覚える。マスクに Anchor があれば、マスクを読む段のあるどの層の出力も変わり得るので、変化の記録の
    /// 通し番号を進め（読む段のブロックの鍵は通し番号を含む。ホストに画素のタイルが無くても、鍵が変わる）、`changed_tiles` が読む層を
    /// 全部返せるよう、マスクが最後に変わった通し番号を持つ。マスクの画素・有効・反転・濃度・フィルター・付け外し・スマートマスクの
    /// 入れ替えは、どれも `mark_layer`（と画素の変化）を通る。
    pub(super) fn note_mask_output(&mut self, index: usize) {
        let l = &self.layers[index];
        if l.mask.as_ref().is_some_and(|m| m.anchor.is_some()) {
            let id = l.id;
            self.journal.serial += 1;
            self.effects.mask_serials.insert(id, self.journal.serial);
        }
    }

    // ───────── 設定の組み立て ─────────

    fn tile_dims(&self) -> (u32, u32) {
        (
            self.width.div_ceil(self.tile_size),
            self.height.div_ceil(self.tile_size),
        )
    }

    fn block_tiles(&self) -> u32 {
        let (cols, rows) = self.tile_dims();
        (self.effects.block_pixels / self.tile_size)
            .max(1)
            .min(cols.max(rows))
    }

    pub(super) fn range_of_rect(&self, rect: Rect) -> TileRange {
        let ts = self.tile_size;
        TileRange {
            x0: rect.x / ts,
            y0: rect.y / ts,
            x1: (rect.x + rect.width).div_ceil(ts),
            y1: (rect.y + rect.height).div_ceil(ts),
        }
    }

    /// 画素の矩形（画布の中）を、半径 margin だけ広げたタイルの範囲に。
    fn grown_range(&self, r: TileRange, margin_pixels: u32) -> TileRange {
        let (cols, rows) = self.tile_dims();
        let m = margin_pixels.div_ceil(self.tile_size);
        TileRange {
            x0: r.x0.saturating_sub(m),
            y0: r.y0.saturating_sub(m),
            x1: (r.x1 + m).min(cols),
            y1: (r.y1 + m).min(rows),
        }
    }

    fn whole_range(&self) -> TileRange {
        let (cols, rows) = self.tile_dims();
        TileRange {
            x0: 0,
            y0: 0,
            x1: cols,
            y1: rows,
        }
    }

    fn make_spec(&self, index: usize, key: SourceKey) -> Spec {
        let layer = &self.layers[index];
        let chain: Vec<FilterEffect> = match key {
            SourceKey::Channel(c) => layer.active_chain(c).into_iter().cloned().collect(),
            SourceKey::Mask => layer
                .mask
                .as_ref()
                .map(|m| {
                    m.filters
                        .iter()
                        .filter(|e| e.is_active())
                        .cloned()
                        .collect()
                })
                .unwrap_or_default(),
        };
        let (value_type, fill) = match key {
            SourceKey::Mask => (ValueType::Mask, None),
            SourceKey::Channel(c) => {
                let kind = self.channel_kind(c).unwrap_or(ChannelKind::Color);
                let fill = (layer.kind == LayerKind::Fill).then(|| {
                    let shape = if layer.is_decal() {
                        layer
                            .fill_images
                            .iter()
                            .next()
                            .filter(|(sc, _)| **sc != c)
                            .map(|(sc, id)| (*sc, *id))
                    } else {
                        None
                    };
                    FillConfig {
                        value: layer.fill.get(&c).copied().unwrap_or(Rgba8::TRANSPARENT),
                        kind,
                        image: layer.fill_images.get(&c).copied(),
                        gradient: layer.fill_gradients.get(&c).cloned(),
                        projection: layer.projection,
                        shape,
                        decal: layer.is_decal(),
                    }
                });
                (value_type_of(kind), fill)
            }
        };
        let points = self.anchor_points();
        let anchors: Vec<Option<AnchorStage>> = chain
            .iter()
            .map(|e| {
                let EffectSettings::Generator(g) = &e.settings else {
                    return None;
                };
                if g.kind != generator::Kind::Anchor {
                    return None;
                }
                Some(match g.anchor.resolve(&points, index, self.layers.len()) {
                    Ok(p) => AnchorStage::Bound(AnchorRef {
                        host: self.layers[p.host].id,
                        placement: match p.placement {
                            anchor::Placement::Layer => AnchorPlacement::Layer,
                            anchor::Placement::Mask => AnchorPlacement::Mask,
                        },
                        channel: g.anchor.channel,
                        read: g.anchor.read,
                    }),
                    Err(issue) => AnchorStage::Unusable(issue),
                })
            })
            .collect();
        let halo = chain.iter().map(|e| e.settings.halo()).sum();
        let global = chain.iter().any(|e| e.settings.is_global());
        let seam = self.seam_shape(chain.iter().map(|e| e.settings.halo()));
        // 継ぎ目をまたぐ評価が読むモデルの UV の位相は、マップ・画像とは別の鍵（`Stamp::topology`）で見る
        let reads_inputs = chain.iter().any(|e| e.settings.is_generator())
            || fill
                .as_ref()
                .is_some_and(|f| f.image.is_some() || f.gradient.is_some() || f.decal);
        let reads_anchor = anchors.iter().any(Option::is_some);
        Spec {
            layer: index,
            id: layer.id,
            key,
            config: Arc::new(Config {
                chain,
                value_type,
                fill,
                anchors,
                seam_band: seam.0,
            }),
            halo,
            global,
            reads_inputs,
            reads_anchor,
            seam,
        }
    }

    fn source_serial_window(&self, id: LayerId, key: SourceKey, range: TileRange) -> u64 {
        let Some(map) = self.effects.source.get(&(id, key)) else {
            return 0;
        };
        if (range.x1 - range.x0) as usize * (range.y1 - range.y0) as usize > map.len() {
            return map
                .iter()
                .filter(|(c, _)| range.contains(**c))
                .map(|(_, s)| *s)
                .max()
                .unwrap_or(0);
        }
        range
            .iter()
            .filter_map(|c| map.get(&c))
            .copied()
            .max()
            .unwrap_or(0)
    }

    fn stamp(&self, spec: &Spec, range: TileRange) -> Stamp {
        // 塗りつぶしは面を読まない。ただしパスのある塗りつぶしの層は、パスの画素（層の面）を最後に重ねるので、面の変化を見る
        let fill_paths = match spec.key {
            SourceKey::Channel(c) => self.layers[spec.layer].fill_paths_draw(c),
            SourceKey::Mask => false,
        };
        let source = if matches!(spec.key, SourceKey::Channel(_))
            && spec.config.fill.is_some()
            && !fill_paths
        {
            0
        } else if spec.global {
            self.source_serial_window(spec.id, spec.key, self.whole_range())
        } else {
            let grown = self.grown_range(range, spec.halo);
            self.source_serial_window(spec.id, spec.key, grown)
                .max(self.seam_source_serial(spec.id, spec.key, spec.seam, spec.halo, grown.iter()))
        };
        Stamp {
            config: spec.config.clone(),
            generation: self.effects.generation,
            source,
            inputs: if spec.reads_inputs {
                self.effects.inputs_revision
            } else {
                0
            },
            topology: if spec.seam.0 > 0 {
                self.effects.topology_revision
            } else {
                0
            },
            // Anchor が読む合成は、どの層の変化でも変わり得る。変化の記録の通し番号が同じなら何も変わっていない
            anchors: if spec.reads_anchor {
                self.journal.serial + 1
            } else {
                0
            },
        }
    }

    // ───────── 評価済みの面の作成（合成が読む） ─────────

    /// 合成のために、プランに出る層のうち評価が要るものの出力を、矩形のタイルの範囲で作る。
    pub(crate) fn evaluate_for_composite(
        &self,
        channel: Channel,
        kind: ChannelKind,
        rect: Rect,
        cancel: Option<&AtomicBool>,
    ) -> Result<EvalSet, CoreError> {
        self.evaluate_for_composite_in(
            channel,
            kind,
            &Region::Range(self.range_of_rect(rect)),
            cancel,
        )
    }

    /// `evaluate_for_composite` の、散らばったタイルの組も渡せる形。
    pub(crate) fn evaluate_for_composite_in(
        &self,
        channel: Channel,
        kind: ChannelKind,
        region: &Region<'_>,
        cancel: Option<&AtomicBool>,
    ) -> Result<EvalSet, CoreError> {
        if !self.layers.iter().any(|l| {
            l.has_evaluated_output(channel)
                || l.mask.as_ref().is_some_and(|m| m.has_active_filters())
        }) {
            return Ok(EvalSet::default());
        }
        let stack = Stack::new(&self.layers, channel, kind, None);
        self.evaluate_entries_in(&stack.plan(), channel, region, cancel)
    }

    /// 計画（段の並び）に出る層のうち評価が要るものの出力を、矩形のタイルの範囲で作る。合成（`evaluate_for_composite`）と、グループの
    /// 出力（そのグループの子の計画）が同じ道を通る。
    pub(super) fn evaluate_entries(
        &self,
        entries: &[Entry],
        channel: Channel,
        rect: Rect,
        cancel: Option<&AtomicBool>,
    ) -> Result<EvalSet, CoreError> {
        self.evaluate_entries_in(
            entries,
            channel,
            &Region::Range(self.range_of_rect(rect)),
            cancel,
        )
    }

    fn evaluate_entries_in(
        &self,
        entries: &[Entry],
        channel: Channel,
        region: &Region<'_>,
        cancel: Option<&AtomicBool>,
    ) -> Result<EvalSet, CoreError> {
        let mut set = EvalSet::default();
        let mut indices = Vec::new();
        collect_layers(entries, &mut indices);
        for i in indices {
            let l = &self.layers[i];
            if matches!(l.kind, LayerKind::Raster | LayerKind::Fill)
                && l.has_evaluated_output(channel)
            {
                set.content.insert(
                    i,
                    self.output_surface_in(i, SourceKey::Channel(channel), region, cancel)?,
                );
            }
            if l.mask
                .as_ref()
                .is_some_and(|m| !m.is_neutral() && m.has_active_filters())
            {
                set.masks.insert(
                    i,
                    self.output_surface_in(i, SourceKey::Mask, region, cancel)?,
                );
            }
        }
        Ok(set)
    }

    /// 効果の出力（フィルター・Generator・画像・グラデーション）を、評価していないタイルがあるか（そのタイルを正確に合成するには、評価が
    /// 要るか）。評価した出力が持てるブロックが、キャッシュに無い・古いとき true。表示が、操作中の効果を粗く見せるかを決めるのに使う。
    pub fn effects_pending(&self, channel: Channel, coords: &[TileCoord]) -> bool {
        let Ok(kind) = self.channel_kind(channel) else {
            return false;
        };
        if !self.layers.iter().any(|l| {
            l.has_evaluated_output(channel)
                || l.mask.as_ref().is_some_and(|m| m.has_active_filters())
        }) {
            return false;
        }
        let stack = Stack::new(&self.layers, channel, kind, None);
        let mut indices = Vec::new();
        collect_layers(&stack.plan(), &mut indices);
        let bt = self.block_tiles();
        let missing = |i: usize, key: SourceKey| -> bool {
            let spec = self.make_spec(i, key);
            let mut blocks: Vec<(u32, u32)> = coords
                .iter()
                .filter(|c| self.may_cover(&spec, **c))
                .map(|c| (c.x / bt, c.y / bt))
                .collect();
            blocks.sort_unstable();
            blocks.dedup();
            blocks
                .iter()
                .any(|&(bx, by)| self.cached_block(&spec, bx, by).is_none())
        };
        indices.into_iter().any(|i| {
            let l = &self.layers[i];
            (matches!(l.kind, LayerKind::Raster | LayerKind::Fill)
                && l.has_evaluated_output(channel)
                && missing(i, SourceKey::Channel(channel)))
                || (l
                    .mask
                    .as_ref()
                    .is_some_and(|m| !m.is_neutral() && m.has_active_filters())
                    && missing(i, SourceKey::Mask))
        })
    }

    /// 合成のために、プランに出る層の評価した出力を、歩幅 stride で粗く評価して作る（タイルの束のタイルだけ。面は文書の 1/歩幅で、
    /// キャッシュには入れない）。評価が要る層が無ければ空。
    pub(crate) fn evaluate_for_composite_coarse(
        &self,
        channel: Channel,
        kind: ChannelKind,
        coords: &[TileCoord],
        stride: u32,
        cancel: Option<&AtomicBool>,
    ) -> Result<EvalSet, CoreError> {
        if !self.layers.iter().any(|l| {
            l.has_evaluated_output(channel)
                || l.mask.as_ref().is_some_and(|m| m.has_active_filters())
        }) {
            return Ok(EvalSet::default());
        }
        let stack = Stack::new(&self.layers, channel, kind, None);
        let mut indices = Vec::new();
        collect_layers(&stack.plan(), &mut indices);
        let mut set = EvalSet {
            reduced: Some(stride),
            ..EvalSet::default()
        };
        for i in indices {
            let l = &self.layers[i];
            if matches!(l.kind, LayerKind::Raster | LayerKind::Fill)
                && l.has_evaluated_output(channel)
            {
                set.content.insert(
                    i,
                    self.coarse_output_surface(
                        i,
                        SourceKey::Channel(channel),
                        coords,
                        stride,
                        cancel,
                    )?,
                );
            }
            if l.mask
                .as_ref()
                .is_some_and(|m| !m.is_neutral() && m.has_active_filters())
            {
                set.masks.insert(
                    i,
                    self.coarse_output_surface(i, SourceKey::Mask, coords, stride, cancel)?,
                );
            }
        }
        Ok(set)
    }

    /// 層の出力の面を、歩幅 stride で粗く評価する（coords のタイルだけ）。読み元を歩幅で拾い、ぼかし・シャープの半径を歩幅で割って、
    /// 縮めた画像の上で評価する（評価にかかる画素数が 1/歩幅²）。操作中の仮の絵で、離したあとの正確な評価の代わりではない。面は幅・高さが
    /// 文書の 1/歩幅、タイルの一辺が文書のタイルの 1/歩幅で、タイルの座標は文書と同じ。
    fn coarse_output_surface(
        &self,
        index: usize,
        key: SourceKey,
        coords: &[TileCoord],
        stride: u32,
        cancel: Option<&AtomicBool>,
    ) -> Result<Surface, CoreError> {
        let spec = self.make_spec(index, key);
        let ct = self.tile_size / stride;
        let (cw, ch) = (self.width.div_ceil(stride), self.height.div_ceil(stride));
        let mut out = Surface::new(cw, ch, ct);
        let wanted: Vec<TileCoord> = coords
            .iter()
            .copied()
            .filter(|c| self.may_cover(&spec, *c))
            .collect();
        if wanted.is_empty() {
            return Ok(out);
        }
        let bt = self.block_tiles();
        let mut groups: HashMap<(u32, u32), Vec<TileCoord>> = HashMap::new();
        for c in wanted {
            groups.entry((c.x / bt, c.y / bt)).or_default().push(c);
        }
        let groups: Vec<Vec<TileCoord>> = groups.into_values().collect();
        let statistics = if spec.global {
            Some(self.coarse_statistics(&spec, stride, cancel)?)
        } else {
            None
        };
        let done: Vec<Result<Vec<(TileCoord, Tile)>, CoreError>> = groups
            .par_iter()
            .map(|tiles| self.coarse_block(&spec, tiles, stride, statistics.as_deref(), cancel))
            .collect();
        for block in done {
            for (coord, tile) in block? {
                out.restore(coord, Some(&tile));
            }
        }
        Ok(out)
    }

    /// 粗い評価の、全域の統計（正規化。縮めた入力の全体から）。
    fn coarse_statistics(
        &self,
        spec: &Spec,
        stride: u32,
        cancel: Option<&AtomicBool>,
    ) -> Result<Arc<Vec<Option<filter::Statistics>>>, CoreError> {
        let full = (self.width, self.height);
        let coarse = (full.0.div_ceil(stride), full.1.div_ceil(stride));
        let seams = self.coarse_seam_table(spec, stride, coarse);
        let stats = self.with_env(spec, self.whole_range(), cancel, |env| {
            let strided = StridedSource {
                inner: env.source,
                stride,
                coarse,
                full,
            };
            let generators = ScaledGenerators {
                inner: env.generators,
                stride,
                full,
            };
            let stages = coarse_stages(env.stages, stride);
            let options = filter::Options {
                working_budget: self.effects.working_budget,
                block_size: self.effects.block_pixels,
                cancel,
                generators: Some(&generators),
                statistics: None,
                seams: seams.as_deref(),
            };
            filter::statistics(&strided, env.value_type, &stages, &options)
                .map_err(map_filter_error)
        })?;
        Ok(Arc::new(stats))
    }

    /// 粗い評価の、縮めた画像の大きさの帯の写し（半径を歩幅で割った近傍の段の最大から。またがないなら None）。
    fn coarse_seam_table(
        &self,
        spec: &Spec,
        stride: u32,
        (width, height): (u32, u32),
    ) -> Option<Arc<crate::geometry::SeamBand>> {
        if spec.seam.0 == 0 {
            return None;
        }
        let max = spec
            .config
            .chain
            .iter()
            .map(|e| (e.settings.halo() + stride / 2) / stride)
            .max()
            .unwrap_or(0);
        let table = self.seam_table_at(width, height, crate::geometry::seam_band_width(max))?;
        // 使うと作業メモリの予算を超えるなら、またがずに 2D で評価する
        let stages = coarse_stages(&spec_stages(spec), stride);
        self.seams_fit(&stages, &table).then_some(table)
    }

    /// 粗い評価の 1 ブロックぶん（tiles は同じブロックの、出力を持ち得るタイル）。
    fn coarse_block(
        &self,
        spec: &Spec,
        tiles: &[TileCoord],
        stride: u32,
        statistics: Option<&Vec<Option<filter::Statistics>>>,
        cancel: Option<&AtomicBool>,
    ) -> Result<Vec<(TileCoord, Tile)>, CoreError> {
        cancelled(cancel)?;
        let ts = self.tile_size;
        let ct = (ts / stride) as usize;
        let full = (self.width, self.height);
        let coarse = (full.0.div_ceil(stride), full.1.div_ceil(stride));
        let bx0 = tiles.iter().map(|c| c.x).min().unwrap_or(0);
        let by0 = tiles.iter().map(|c| c.y).min().unwrap_or(0);
        let bx1 = tiles.iter().map(|c| c.x).max().unwrap_or(0) + 1;
        let by1 = tiles.iter().map(|c| c.y).max().unwrap_or(0) + 1;
        // 評価する粗い画素の矩形（タイルを含む最小の矩形。端は文書の端で切る）
        let (cx0, cy0) = (bx0 * ts / stride, by0 * ts / stride);
        let (cx1, cy1) = (
            (bx1 * ts).min(full.0).div_ceil(stride),
            (by1 * ts).min(full.1).div_ceil(stride),
        );
        let region = Rect::new(cx0, cy0, cx1 - cx0, cy1 - cy0);
        let range = TileRange {
            x0: bx0,
            y0: by0,
            x1: bx1,
            y1: by1,
        };
        let seams = self.coarse_seam_table(spec, stride, coarse);
        let output = self.with_env(spec, self.grown_range(range, spec.halo), cancel, |env| {
            let strided = StridedSource {
                inner: env.source,
                stride,
                coarse,
                full,
            };
            let generators = ScaledGenerators {
                inner: env.generators,
                stride,
                full,
            };
            let stages = coarse_stages(env.stages, stride);
            let options = filter::Options {
                working_budget: self.effects.working_budget,
                block_size: self.effects.block_pixels,
                cancel,
                generators: Some(&generators),
                statistics: statistics.map(Vec::as_slice),
                seams: seams.as_deref(),
            };
            filter::evaluate(&strided, env.value_type, &stages, region, &options)
                .map_err(map_filter_error)
        })?;
        let row = region.width as usize * 4;
        let mut out = Vec::with_capacity(tiles.len());
        for &coord in tiles {
            let mut bytes = vec![0u8; ct * ct * 4];
            let (x0, y0) = (coord.x as usize * ct, coord.y as usize * ct);
            let w = ct.min(coarse.0 as usize - x0);
            let h = ct.min(coarse.1 as usize - y0);
            for r in 0..h {
                let src = (y0 + r - region.y as usize) * row + (x0 - region.x as usize) * 4;
                bytes[r * ct * 4..][..w * 4].copy_from_slice(&output[src..src + w * 4]);
            }
            out.push((coord, covered_tile(bytes)));
        }
        Ok(out)
    }

    /// 合成のためでなく、層の並びの写し（結合の準備のために並べ直した層）の、評価した出力を作る。鍵は `layers` の番号で、
    /// `doc_index[k]` は `layers[k]` のこの文書での番号（評価は文書の層の設定・Anchor・入力・キャッシュで行うので、写しの番号ではなく
    /// 文書の番号で引く）。`layers` を下から上へ並べた計画に出る層だけ、全面を評価する。評価が要る層が無ければ空。
    pub(crate) fn evaluate_slice(
        &self,
        layers: &[crate::layer::Layer],
        doc_index: &[usize],
        channel: Channel,
        kind: ChannelKind,
        cancel: Option<&AtomicBool>,
    ) -> Result<EvalSet, CoreError> {
        debug_assert_eq!(layers.len(), doc_index.len());
        let mut set = EvalSet::default();
        if !layers.iter().any(|l| {
            l.has_evaluated_output(channel)
                || l.mask.as_ref().is_some_and(|m| m.has_active_filters())
        }) {
            return Ok(set);
        }
        let stack = Stack::new(layers, channel, kind, None);
        let mut indices = Vec::new();
        collect_layers(&stack.plan(), &mut indices);
        let range = self.range_of_rect(Rect::new(0, 0, self.width, self.height));
        for k in indices {
            let l = &layers[k];
            if matches!(l.kind, LayerKind::Raster | LayerKind::Fill)
                && l.has_evaluated_output(channel)
            {
                set.content.insert(
                    k,
                    self.output_surface(doc_index[k], SourceKey::Channel(channel), range, cancel)?,
                );
            }
            if l.mask
                .as_ref()
                .is_some_and(|m| !m.is_neutral() && m.has_active_filters())
            {
                set.masks.insert(
                    k,
                    self.output_surface(doc_index[k], SourceKey::Mask, range, cancel)?,
                );
            }
        }
        Ok(set)
    }

    /// 層の（フィルター・投影を通した）出力の面を、タイルの範囲だけ。評価済みのブロックを使い回し、足りないブロックは作業メモリの
    /// 予算に収まる数ずつ並べて評価する（結果は並びによらない）。
    pub(super) fn output_surface(
        &self,
        index: usize,
        key: SourceKey,
        range: TileRange,
        cancel: Option<&AtomicBool>,
    ) -> Result<Surface, CoreError> {
        self.output_surface_in(index, key, &Region::Range(range), cancel)
    }

    /// `output_surface` の、散らばったタイルの組も渡せる形（組のタイルを含むブロックだけを評価する）。
    pub(super) fn output_surface_in(
        &self,
        index: usize,
        key: SourceKey,
        region: &Region<'_>,
        cancel: Option<&AtomicBool>,
    ) -> Result<Surface, CoreError> {
        let spec = self.make_spec(index, key);
        let bt = self.block_tiles();
        let blocks: Vec<(u32, u32)> = match region {
            Region::Range(range) => (range.y0 / bt..=(range.y1 - 1) / bt)
                .flat_map(|by| (range.x0 / bt..=(range.x1 - 1) / bt).map(move |bx| (bx, by)))
                .collect(),
            Region::Tiles(tiles) => {
                let mut set: Vec<(u32, u32)> = tiles.iter().map(|c| (c.x / bt, c.y / bt)).collect();
                set.sort_by_key(|&(bx, by)| (by, bx));
                set.dedup();
                set
            }
        };
        let mut results: Vec<Option<BlockTiles>> = blocks
            .iter()
            .map(|&(bx, by)| self.cached_block(&spec, bx, by))
            .collect();
        let missing: Vec<usize> = (0..blocks.len())
            .filter(|&i| results[i].is_none())
            .collect();
        if !missing.is_empty() {
            self.prepare_shared(&spec, cancel)?;
        }
        let parallel = if missing.is_empty() {
            1
        } else {
            self.parallel_blocks(&spec)
        };
        for batch in missing.chunks(parallel) {
            cancelled(cancel)?;
            let done: Vec<Result<BlockTiles, CoreError>> = batch
                .par_iter()
                .map(|&i| self.compute_block(&spec, blocks[i].0, blocks[i].1, cancel))
                .collect();
            for (&i, r) in batch.iter().zip(done) {
                results[i] = Some(r?);
            }
        }
        let mut out = Surface::new(self.width, self.height, self.tile_size);
        for tiles in results.into_iter().flatten() {
            for (coord, tile) in tiles.iter() {
                // 範囲の外のタイルは入れない（散らばった組では、評価したブロックの中の組でないタイルも、読まれないので入れて構わない）
                let wanted = match region {
                    Region::Range(range) => range.contains(*coord),
                    Region::Tiles(_) => true,
                };
                if wanted {
                    if let Some(t) = tile {
                        out.restore(*coord, Some(t));
                    }
                }
            }
        }
        Ok(out)
    }

    /// どのブロックの評価も使う、全域の統計（正規化）と画像のミップマップを、ブロックを並べて評価する前に 1 回ずつ作ってキャッシュへ入れる。
    /// 並べてから各ブロックが取りに行くと、キャッシュが空の最初のバッチでは全ブロックが同じものを同時に作り、作業メモリが予算の
    /// 並列数倍に届き（統計は呼ぶたびに予算いっぱいまで使う）、画像のミップマップも重複して作る。
    fn prepare_shared(&self, spec: &Spec, cancel: Option<&AtomicBool>) -> Result<(), CoreError> {
        // 継ぎ目をまたぐ帯の写しも、ブロックを並べる前に 1 回だけ作る（作れなければ、どのブロックも 2D で評価する）
        let _ = self.seam_table(spec.seam.0);
        if spec.global {
            self.stage_statistics(spec, cancel)?;
        }
        if let Some(f) = &spec.config.fill {
            if let Some(id) = f.image {
                let _ = self.mip_chain(id, f.kind);
            }
            if let Some((sc, id)) = f.shape {
                if let Ok(kind) = self.channel_kind(sc) {
                    let _ = self.mip_chain(id, kind);
                }
            }
        }
        Ok(())
    }

    /// 同時に評価してよいブロックの数: 作業メモリの予算に収まる数を、並列の数で頭打ちにする（最小 1）。
    fn parallel_blocks(&self, spec: &Spec) -> usize {
        let stages = spec_stages(spec);
        let side = (self.effects.block_pixels / self.tile_size).max(1) * self.tile_size;
        let output = u64::from(side.min(self.width)) * u64::from(side.min(self.height)) * 4;
        let mut working = filter::block_working_bytes(
            &stages,
            self.effects.block_pixels,
            self.width,
            self.height,
        )
        .unwrap_or(0);
        // 継ぎ目をまたいで評価する（帯の写しがあって、使っても予算に収まる）なら、その分も。帯の写しは `prepare_shared` が先に作ってある
        if let Some(table) = self.seam_table_for(spec) {
            working += filter::seam_working_bytes(
                &stages,
                self.effects.block_pixels,
                self.width,
                self.height,
                table.texel_count() as u64,
            );
        }
        let need = working.saturating_add(output).max(1);
        ((self.effects.working_budget / need) as usize)
            .clamp(1, rayon::current_num_threads().max(1))
    }

    fn block_range(&self, bx: u32, by: u32) -> TileRange {
        let bt = self.block_tiles();
        let (cols, rows) = self.tile_dims();
        TileRange {
            x0: bx * bt,
            y0: by * bt,
            x1: ((bx + 1) * bt).min(cols),
            y1: ((by + 1) * bt).min(rows),
        }
    }

    /// キャッシュにあって鍵が合うブロック（無ければ None）。
    fn cached_block(&self, spec: &Spec, bx: u32, by: u32) -> Option<BlockTiles> {
        let stamp = self.stamp(spec, self.block_range(bx, by));
        let key = BlockKey {
            layer: spec.id,
            key: spec.key,
            bx,
            by,
        };
        let mut c = self.cache();
        c.clock += 1;
        let clock = c.clock;
        let e = c.blocks.get_mut(&key)?;
        if e.stamp == stamp {
            e.used = clock;
            Some(e.tiles.clone())
        } else {
            None
        }
    }

    /// ブロックを評価して、予算に収まればキャッシュに持つ。
    fn compute_block(
        &self,
        spec: &Spec,
        bx: u32,
        by: u32,
        cancel: Option<&AtomicBool>,
    ) -> Result<BlockTiles, CoreError> {
        let range = self.block_range(bx, by);
        let stamp = self.stamp(spec, range);
        let key = BlockKey {
            layer: spec.id,
            key: spec.key,
            bx,
            by,
        };
        let tiles: BlockTiles = Arc::new(self.evaluate_block(spec, range, cancel)?);
        let bytes: u64 = tiles
            .iter()
            .map(|(_, t)| t.as_ref().map_or(0, Tile::byte_size))
            .sum();
        {
            let mut c = self.cache();
            c.counters.blocks_evaluated += 1;
            if self.effects.cache_budget >= bytes && self.effects.cache_budget > 0 {
                c.clock += 1;
                let used = c.clock;
                if let Some(old) = c.blocks.remove(&key) {
                    c.bytes -= old.bytes;
                }
                c.bytes += bytes;
                c.blocks.insert(
                    key,
                    BlockEntry {
                        stamp,
                        tiles: tiles.clone(),
                        bytes,
                        used,
                    },
                );
            }
        }
        self.trim_effect_cache();
        Ok(tiles)
    }

    /// ブロックの出力（キャッシュにあって鍵が合えばそれ、無ければ評価して持つ）。
    fn block_output(
        &self,
        spec: &Spec,
        bx: u32,
        by: u32,
        cancel: Option<&AtomicBool>,
    ) -> Result<BlockTiles, CoreError> {
        match self.cached_block(spec, bx, by) {
            Some(t) => Ok(t),
            None => self.compute_block(spec, bx, by, cancel),
        }
    }

    /// 層のそのタイルの出力（評価した出力か、保存した画素。無ければ None）。Anchor の合成が読む。
    fn layer_output_tile(
        &self,
        index: usize,
        channel: Channel,
        coord: TileCoord,
        cancel: Option<&AtomicBool>,
    ) -> Result<Option<Tile>, CoreError> {
        let l = &self.layers[index];
        if matches!(l.kind, LayerKind::Raster | LayerKind::Fill) && l.has_evaluated_output(channel)
        {
            let spec = self.make_spec(index, SourceKey::Channel(channel));
            let bt = self.block_tiles();
            let tiles = self.block_output(&spec, coord.x / bt, coord.y / bt, cancel)?;
            return Ok(tiles
                .iter()
                .find(|(c, _)| *c == coord)
                .and_then(|(_, t)| t.clone()));
        }
        Ok(match l.kind {
            LayerKind::Raster => l.surface(channel).and_then(|s| s.tile(coord).cloned()),
            _ => None,
        })
    }

    /// 層のマスクのそのタイルの出力（フィルターを通した隠す量。無ければ None）。
    fn mask_output_tile(
        &self,
        index: usize,
        coord: TileCoord,
        cancel: Option<&AtomicBool>,
    ) -> Result<Option<Tile>, CoreError> {
        let Some(m) = &self.layers[index].mask else {
            return Ok(None);
        };
        if m.has_active_filters() {
            let spec = self.make_spec(index, SourceKey::Mask);
            let bt = self.block_tiles();
            let tiles = self.block_output(&spec, coord.x / bt, coord.y / bt, cancel)?;
            return Ok(tiles
                .iter()
                .find(|(c, _)| *c == coord)
                .and_then(|(_, t)| t.clone()));
        }
        Ok(m.surface.tile(coord).cloned())
    }

    /// 層の評価済みの出力の画素（フィルター・投影を通した、マスク・不透明度・合成の前の値）。
    pub fn layer_output_pixel(
        &self,
        id: LayerId,
        channel: Channel,
        x: u32,
        y: u32,
    ) -> Result<Rgba8, CoreError> {
        let index = self.index_of(id)?;
        self.require_channel(channel)?;
        if x >= self.width || y >= self.height {
            return Err(CoreError::InvalidArgument("画素がキャンバスの外"));
        }
        let ts = self.tile_size;
        let coord = TileCoord::new(x / ts, y / ts);
        let l = &self.layers[index];
        if !matches!(l.kind, LayerKind::Raster | LayerKind::Fill) {
            return Ok(Rgba8::TRANSPARENT);
        }
        if l.has_evaluated_output(channel) {
            return self
                .layer_output_tile(index, channel, coord, None)?
                .map_or(Ok(Rgba8::TRANSPARENT), |t| {
                    t.get((((y % ts) * ts + x % ts) * 4) as usize)
                });
        }
        l.pixel(channel, x, y)
    }

    /// マスクの評価済みの出力の隠す量（0〜255。フィルターを通した、有効・反転・濃度の前の値）。
    pub fn mask_output_hide(&self, id: LayerId, x: u32, y: u32) -> Result<u8, CoreError> {
        let index = self.index_of(id)?;
        if x >= self.width || y >= self.height {
            return Err(CoreError::InvalidArgument("画素がキャンバスの外"));
        }
        if self.layers[index].mask.is_none() {
            return Err(CoreError::Unsupported("層にマスクが無い"));
        }
        let ts = self.tile_size;
        self.mask_output_tile(index, TileCoord::new(x / ts, y / ts), None)?
            .map_or(Ok(0), |t| {
                t.get((((y % ts) * ts + x % ts) * 4) as usize).map(|p| p.a)
            })
    }

    // ───────── ブロックの評価 ─────────

    /// タイルが評価の出力を持ち得るか（C# の MayCover）。持ち得ない所は無し（透明）にする。
    fn may_cover(&self, spec: &Spec, coord: TileCoord) -> bool {
        let layer = &self.layers[spec.layer];
        let chain = &spec.config.chain;
        let near = |surface: Option<&Surface>, reach_pixels: u32| -> bool {
            let Some(s) = surface else { return false };
            if s.tile_count() == 0 {
                return false;
            }
            let r = self.grown_range(
                TileRange {
                    x0: coord.x,
                    y0: coord.y,
                    x1: coord.x + 1,
                    y1: coord.y + 1,
                },
                reach_pixels,
            );
            r.iter().any(|c| s.has_tile(c))
        };
        match spec.key {
            SourceKey::Mask => {
                let surface = layer.mask.as_ref().map(|m| &m.surface);
                if !chain.iter().all(|e| e.settings.preserves_zero()) {
                    return true; // 反転・ノイズなどは、何も無い所にも値を作る
                }
                near(surface, spec.halo)
                    || self.seam_reaches_content(spec.seam, spec.halo, coord, surface)
            }
            SourceKey::Channel(c) => {
                if let Some(f) = &spec.config.fill {
                    if f.image.is_some() || f.decal {
                        return true;
                    }
                    // パスの画素（層の面）のあるタイルも
                    return f.gradient.is_some()
                        || f.value != Rgba8::TRANSPARENT
                        || (layer.fill_paths_draw(c) && near(layer.surface(c), 0));
                }
                let expansion: u32 = chain
                    .iter()
                    .filter(|e| e.settings.expands_coverage())
                    .map(|e| e.settings.halo())
                    .sum();
                near(layer.surface(c), expansion)
                    || self.seam_reaches_content(spec.seam, expansion, coord, layer.surface(c))
            }
        }
    }

    fn evaluate_block(
        &self,
        spec: &Spec,
        range: TileRange,
        cancel: Option<&AtomicBool>,
    ) -> Result<Vec<(TileCoord, Option<Tile>)>, CoreError> {
        cancelled(cancel)?;
        let ts = self.tile_size;
        let coords: Vec<TileCoord> = range.iter().collect();
        let covered: Vec<TileCoord> = coords
            .iter()
            .copied()
            .filter(|c| self.may_cover(spec, *c))
            .collect();
        if covered.is_empty() {
            return Ok(coords.into_iter().map(|c| (c, None)).collect());
        }
        // 評価する画素の矩形: 評価するタイルを含む最小の矩形（画布の中）
        let bx0 = covered.iter().map(|c| c.x).min().unwrap_or(0);
        let by0 = covered.iter().map(|c| c.y).min().unwrap_or(0);
        let bx1 = covered.iter().map(|c| c.x).max().unwrap_or(0) + 1;
        let by1 = covered.iter().map(|c| c.y).max().unwrap_or(0) + 1;
        let region = Rect::new(
            bx0 * ts,
            by0 * ts,
            (bx1 * ts).min(self.width) - bx0 * ts,
            (by1 * ts).min(self.height) - by0 * ts,
        );
        let tiles = TileRange {
            x0: bx0,
            y0: by0,
            x1: bx1,
            y1: by1,
        };
        let statistics = if spec.global {
            Some(self.stage_statistics(spec, cancel)?)
        } else {
            None
        };
        let seams = self.seam_table_for(spec);
        let output = self.with_env(spec, self.grown_range(tiles, spec.halo), cancel, |env| {
            let options = filter::Options {
                working_budget: self.effects.working_budget,
                block_size: self.effects.block_pixels,
                cancel,
                generators: Some(env.generators),
                statistics: statistics.as_deref().map(Vec::as_slice),
                seams: seams.as_deref(),
            };
            filter::evaluate(env.source, env.value_type, env.stages, region, &options)
                .map_err(map_filter_error)
        })?;
        // 塗りつぶしの層のパス: 塗りつぶしと効果のスタックの結果の上に、パスの画素（層の面）を重ねる
        let mut output = output;
        if let SourceKey::Channel(c) = spec.key {
            let layer = &self.layers[spec.layer];
            if layer.fill_paths_draw(c) {
                if let Some(surface) = layer.surface(c) {
                    over_surface(&mut output, region, surface)?;
                }
            }
        }
        // 評価した画素を、タイルへ切り出す。評価するタイルで全部 0 なら「何も無いが評価した」印の一様な透明にする
        let row = region.width as usize * 4;
        let mut out = Vec::with_capacity(coords.len());
        for coord in coords {
            if !covered.contains(&coord) {
                out.push((coord, None));
                continue;
            }
            let mut bytes = vec![0u8; (ts * ts * 4) as usize];
            let (x0, y0) = (coord.x * ts, coord.y * ts);
            let w = ts.min(self.width - x0) as usize;
            let h = ts.min(self.height - y0) as usize;
            for r in 0..h {
                let src = (y0 as usize + r - region.y as usize) * row
                    + (x0 as usize - region.x as usize) * 4;
                bytes[r * ts as usize * 4..][..w * 4].copy_from_slice(&output[src..src + w * 4]);
            }
            out.push((coord, Some(covered_tile(bytes))));
        }
        Ok(out)
    }

    /// 正規化などの全域の統計（段ごと）。入力・設定・Generator の値が変わるまで持つ。
    fn stage_statistics(
        &self,
        spec: &Spec,
        cancel: Option<&AtomicBool>,
    ) -> Result<Arc<Vec<Option<filter::Statistics>>>, CoreError> {
        let stamp = self.stamp(spec, self.whole_range());
        {
            let c = self.cache();
            if let Some(e) = c.stats.get(&(spec.id, spec.key)) {
                if e.stamp == stamp {
                    return Ok(e.stats.clone());
                }
            }
        }
        let seams = self.seam_table_for(spec);
        let stats = self.with_env(spec, self.whole_range(), cancel, |env| {
            let options = filter::Options {
                working_budget: self.effects.working_budget,
                block_size: self.effects.block_pixels,
                cancel,
                generators: Some(env.generators),
                statistics: None,
                seams: seams.as_deref(),
            };
            filter::statistics(env.source, env.value_type, env.stages, &options)
                .map_err(map_filter_error)
        })?;
        let stats = Arc::new(stats);
        let mut c = self.cache();
        c.counters.statistics_computed += 1;
        c.stats.insert(
            (spec.id, spec.key),
            StatsEntry {
                stamp,
                stats: stats.clone(),
            },
        );
        Ok(stats)
    }

    /// 評価の環境を組んで body を呼ぶ。Anchor を読む段は、anchor_tiles のタイルの Anchor の値を先に作って持つ。
    fn with_env<R>(
        &self,
        spec: &Spec,
        anchor_tiles: TileRange,
        cancel: Option<&AtomicBool>,
        body: impl FnOnce(&Env<'_>) -> Result<R, CoreError>,
    ) -> Result<R, CoreError> {
        let layer = &self.layers[spec.layer];
        let cfg = &*spec.config;
        let inputs = &self.effects.inputs;
        let dims = (self.width, self.height);
        let scalar = cfg.value_type != ValueType::Color;
        let maps: Vec<generator::Map<'_>> = inputs.maps.iter().map(|m| m.as_generator()).collect();
        let frame = inputs.frame.and_then(|f| f.for_generator().ok());

        // Anchor を読む段の、読むタイルの並び
        let mut grids: Vec<Option<(TileGrid, AnchorRef)>> = Vec::with_capacity(cfg.chain.len());
        for stage in &cfg.anchors {
            grids.push(match stage {
                Some(AnchorStage::Bound(r)) => {
                    Some((self.anchor_grid(r, anchor_tiles, cancel)?, r.clone()))
                }
                _ => None,
            });
        }
        let mut values: Vec<Option<AnchorValue<'_>>> = Vec::with_capacity(grids.len());
        for (k, g) in grids.iter().enumerate() {
            values.push(match g {
                None => None,
                Some((grid, r)) => {
                    let EffectSettings::Generator(gs) = &cfg.chain[k].settings else {
                        unreachable!("Anchor の段はジェネレーター");
                    };
                    match r.placement {
                        AnchorPlacement::Layer => {
                            let kind = self
                                .channel_kind(gs.anchor.channel)
                                .map_err(|_| CoreError::ChannelNotFound)?;
                            let read = gs.anchor.read_for(kind).map_err(map_generator_error)?;
                            Some(AnchorValue::Layer(anchor::LayerSample {
                                source: grid,
                                read,
                            }))
                        }
                        AnchorPlacement::Mask => {
                            let host = self.layer(r.host).and_then(|l| l.mask.as_ref());
                            let m = host.ok_or(CoreError::Unsupported("マスクが無い"))?;
                            let mask = anchor::Mask {
                                source: grid,
                                enabled: m.enabled,
                                inverted: m.inverted,
                                density: m.density,
                            };
                            Some(AnchorValue::Mask(
                                anchor::MaskSample::new(mask).map_err(map_generator_error)?,
                            ))
                        }
                    }
                }
            });
        }

        // 画像の段の画像（ミップマップと、投影を束縛したサンプラー。束縛した Generator より長く生きる）。読めない画像の段は入力のまま
        let image_chains: Vec<Option<Arc<ImageMipChain<'static>>>> = cfg
            .chain
            .iter()
            .map(|e| match &e.settings {
                EffectSettings::Generator(g)
                    if g.kind == generator::Kind::Image && g.image.image != 0 =>
                {
                    self.generator_image_chain(g.image.image, scalar).ok()
                }
                _ => None,
            })
            .collect();
        let mut samplers: Vec<Option<FillSampler<'_>>> = Vec::with_capacity(cfg.chain.len());
        for (e, chain) in cfg.chain.iter().zip(&image_chains) {
            samplers.push(match (&e.settings, chain) {
                // 組めない投影（極端な位置の箱など）の段は、画像を待つ段のまま入力を通す（理由は `generator_status` が言う）
                (EffectSettings::Generator(g), Some(chain)) => {
                    self.image_sampler(&g.image.projection, chain).ok()
                }
                _ => None,
            });
        }

        // 段と束縛した Generator
        let mut stages = Vec::with_capacity(cfg.chain.len());
        let mut bound: Vec<Option<BoundGenerator<'_>>> = Vec::with_capacity(cfg.chain.len());
        for (k, e) in cfg.chain.iter().enumerate() {
            match &e.settings {
                EffectSettings::Filter(f) => {
                    stages.push(filter::Stage {
                        settings: f.clone(),
                        enabled: true,
                        strength: e.strength,
                    });
                    bound.push(None);
                }
                EffectSettings::Generator(g) => {
                    stages.push(filter::Stage {
                        settings: filter::Settings::Generator {
                            slot: k as u32,
                            blend: super::effects::generator_blend(g.blend),
                        },
                        enabled: true,
                        strength: e.strength,
                    });
                    let value_source: Result<&dyn anchor::ValueSource, anchor::Issue> =
                        match (&cfg.anchors[k], &values[k]) {
                            (Some(AnchorStage::Unusable(issue)), _) => Err(issue.clone()),
                            (_, Some(v)) => Ok(v),
                            _ => Err(anchor::Issue::NotChosen),
                        };
                    let b = BoundGenerator::bind(g, &maps, frame, dims, value_source).ok();
                    bound.push(match (b, &samplers[k]) {
                        (Some(b), Some(sampler)) => Some(b.with_image(sampler)),
                        (b, _) => b,
                    });
                }
            }
        }
        let stage_input = StageInput { bound, scalar };

        // 塗りつぶしの画像のミップマップ（読み元より長く生きる）
        let (own, shape) = match &cfg.fill {
            Some(f) => (
                f.image.and_then(|id| self.mip_chain(id, f.kind).ok()),
                f.shape.and_then(|(sc, id)| {
                    self.channel_kind(sc)
                        .ok()
                        .and_then(|k| self.mip_chain(id, k).ok())
                }),
            ),
            None => (None, None),
        };
        // 読み元
        let source = match &cfg.fill {
            None => {
                let surface = match spec.key {
                    SourceKey::Channel(c) => layer.surface(c),
                    SourceKey::Mask => layer.mask.as_ref().map(|m| &m.surface),
                };
                ChainSource::Surface(SurfaceSource {
                    surface,
                    width: self.width,
                    height: self.height,
                    failed: OnceLock::new(),
                })
            }
            Some(f) => {
                let gradient = f.gradient.as_ref().and_then(|g| {
                    BoundGenerator::bind(g, &maps, frame, dims, Err(anchor::Issue::NotChosen)).ok()
                });
                let projected = f.image.is_some() || f.decal;
                let sampler = if projected {
                    let position = inputs.map(MapKind::Position);
                    let normal = inputs.map(MapKind::WorldNormal);
                    let input = FillInput {
                        projection: f.projection,
                        width: self.width,
                        height: self.height,
                        image: own.as_deref(),
                        shape: shape.as_deref(),
                        fallback: f.value,
                        missing_image: f.decal && f.image.is_some() && own.is_none(),
                        positions: usable_map(position).map(|m| m.as_fill()),
                        normals: usable_map(normal).map(|m| m.as_fill()),
                        bounds_min: position.map_or([0.; 3], |m| m.bounds_min),
                        bounds_max: position.map_or([1.; 3], |m| m.bounds_max),
                        frame: inputs.frame.map(|fr| fr.for_fill()),
                        stale_position: position.is_some_and(|m| m.state != MapState::Current),
                        stale_normal: normal.is_some_and(|m| m.state != MapState::Current),
                    };
                    Some(FillSampler::bind(input).map_err(map_fill_error)?)
                } else {
                    None
                };
                ChainSource::Fill(Box::new(FillSource {
                    value: f.value,
                    width: self.width,
                    height: self.height,
                    gradient,
                    scalar: f.kind != ChannelKind::Color,
                    sampler,
                    decal: f.decal,
                }))
            }
        };
        let result = body(&Env {
            source: &source,
            value_type: cfg.value_type,
            stages: &stages,
            generators: &stage_input,
        });
        // 読み元のタイルをディスクから読めなかった評価は使わない（キャッシュにも入れない）
        if let ChainSource::Surface(s) = &source {
            if let Some(e) = s.failed.get() {
                return Err(e.clone());
            }
        }
        result
    }

    /// 画像のミップマップ（変換と輝度はチャンネルの種類で決まる。同じ中身・変換・輝度は 1 つを共有し、予算で古いものから捨てる）。
    fn mip_chain(
        &self,
        id: ImageId,
        kind: ChannelKind,
    ) -> Result<Arc<ImageMipChain<'static>>, InactiveReason> {
        self.mip_chain_as(id, kind == ChannelKind::Color, kind == ChannelKind::Scalar)
    }

    /// 画像の段が読む画像のミップマップ（色の対象は色として読む: リニアの画像は sRGB に直す。マスク・スカラーは値のまま。輝度には直さない。
    /// 成分は段が選ぶ）。画像が入力に無いときは `MissingImage`。
    fn generator_image_chain(
        &self,
        id: u128,
        scalar: bool,
    ) -> Result<Arc<ImageMipChain<'static>>, InactiveReason> {
        if !self.effects.inputs.images.contains_key(&ImageId(id)) {
            return Err(InactiveReason::Generator(generator::Inactive::MissingImage));
        }
        self.mip_chain_as(ImageId(id), !scalar, false)
    }

    /// `color` は色として読む（リニアの画像を sRGB に直す）か、`luminance` は輝度に直すか。
    fn mip_chain_as(
        &self,
        id: ImageId,
        color: bool,
        luminance: bool,
    ) -> Result<Arc<ImageMipChain<'static>>, InactiveReason> {
        let image = self
            .effects
            .inputs
            .images
            .get(&id)
            .ok_or_else(|| InactiveReason::Rejected("プロジェクトにその画像が無い".into()))?;
        let conversion = if color && image.color_space == crate::brush::ImageColorSpace::Linear {
            Conversion::LinearToSrgb
        } else {
            Conversion::None
        };
        let key = (image.hash.clone(), conversion as u8, luminance);
        {
            let mut c = self.cache();
            c.clock += 1;
            let clock = c.clock;
            if let Some(e) = c.chains.get_mut(&key) {
                e.used = clock;
                return Ok(e.chain.clone());
            }
        }
        let budget = self.effects.image_cache_budget;
        let chain = ImageMipChain::build_shared(
            image.pixels.clone(),
            image.width,
            image.height,
            conversion,
            luminance,
            budget,
            None,
        )
        .map_err(|e| InactiveReason::Rejected(e.to_string()))?;
        let chain = Arc::new(chain);
        {
            let mut c = self.cache();
            c.counters.mip_chains_built += 1;
            c.clock += 1;
            let used = c.clock;
            c.chain_bytes += chain.bytes();
            c.chains.insert(
                key,
                ChainEntry {
                    chain: chain.clone(),
                    used,
                },
            );
        }
        self.trim_effect_cache();
        Ok(chain)
    }

    // ───────── Anchor ─────────

    /// Anchor が読むタイルの並び（範囲のタイルの、層なら合成したチャンネルの結果・マスクなら評価した隠す量）。
    fn anchor_grid(
        &self,
        r: &AnchorRef,
        range: TileRange,
        cancel: Option<&AtomicBool>,
    ) -> Result<TileGrid, CoreError> {
        let host = self.layer_index(r.host).ok_or(CoreError::LayerNotFound)?;
        let coords: Vec<TileCoord> = range.iter().collect();
        // タイルごとに独立に合成する（結果は並びによらない）。下の層の評価済みのブロックは、キャッシュを通して共有する
        let tiles = coords
            .par_iter()
            .map(|&coord| -> Result<Option<Arc<Vec<u8>>>, CoreError> {
                cancelled(cancel)?;
                Ok(match r.placement {
                    AnchorPlacement::Layer => {
                        self.anchor_layer_tile(host, r.channel, coord, cancel)?
                    }
                    AnchorPlacement::Mask => match self.mask_output_tile(host, coord, cancel)? {
                        None => None,
                        Some(t) => Some(match t.read()? {
                            Pixels::Data(d) => d,
                            uniform => {
                                let mut bytes =
                                    vec![0u8; (self.tile_size * self.tile_size * 4) as usize];
                                uniform.copy_to(&mut bytes);
                                Arc::new(bytes)
                            }
                        }),
                    },
                })
            })
            .collect::<Result<Vec<_>, CoreError>>()?;
        Ok(TileGrid {
            tiles,
            range,
            tile_size: self.tile_size,
            dims: (self.width, self.height),
        })
    }

    /// 層の Anchor のタイル: その層までのスタックを、その層より上が無いものとして合成した結果（全部透明なら None）。
    fn anchor_layer_tile(
        &self,
        host: usize,
        channel: Channel,
        coord: TileCoord,
        cancel: Option<&AtomicBool>,
    ) -> Result<Option<Arc<Vec<u8>>>, CoreError> {
        let key = (self.layers[host].id, channel, coord);
        // 変化の記録の通し番号（どの層のどの変化でも進む）と世代が同じなら、合成は同じ
        let stamp = (self.journal.serial, self.effects.generation);
        {
            let mut c = self.cache();
            c.clock += 1;
            let clock = c.clock;
            if let Some(e) = c.anchors.get_mut(&key) {
                if e.stamp == stamp {
                    e.used = clock;
                    return Ok(e.tile.clone());
                }
            }
        }
        let kind = self.channel_kind(channel)?;
        let ts = self.tile_size;
        let dims = (self.width, self.height);
        // 合成が読む層は、host とそれより下（host の下の兄弟、祖先の下の兄弟、host がグループなら中身）だけで、どれも番号が host 以下
        // （グループは中身の上に並ぶ）。host より上の兄弟は読まない: 評価すると、それが host の Anchor を読むときに host の Anchor へ戻って
        // 終わらない。祖先のグループは画素が要らないので、読まれない層と同じ Group の入れ物にする
        let position: HashMap<LayerId, usize> = self
            .layers
            .iter()
            .enumerate()
            .map(|(i, l)| (l.id, i))
            .collect();
        // 層ごとの入力（読まれる層だけ画素を用意する）
        let mut views: Vec<Option<TileView>> = Vec::with_capacity(self.layers.len());
        let mut mask_views: Vec<Option<TileView>> = Vec::with_capacity(self.layers.len());
        for (i, l) in self.layers.iter().enumerate() {
            let reachable = i <= host;
            let content = if reachable
                && matches!(l.kind, LayerKind::Raster | LayerKind::Fill)
                && (l.kind == LayerKind::Raster || l.has_evaluated_output(channel))
                && l.is_channel_enabled(channel)
                && l.has_content(channel, true)
            {
                Some(TileView {
                    coord,
                    tile_size: ts,
                    dims,
                    tile: self
                        .layer_output_tile(i, channel, coord, cancel)?
                        .map(|t| t.read())
                        .transpose()?,
                })
            } else {
                None
            };
            views.push(content);
            let mask = if reachable && l.mask.as_ref().is_some_and(|m| !m.is_neutral()) {
                Some(TileView {
                    coord,
                    tile_size: ts,
                    dims,
                    tile: self
                        .mask_output_tile(i, coord, cancel)?
                        .map(|t| t.read())
                        .transpose()?,
                })
            } else {
                None
            };
            mask_views.push(mask);
        }
        let empty = TileView {
            coord,
            tile_size: ts,
            dims,
            tile: None,
        };
        let layers: Vec<anchor::Layer<'_>> = self
            .layers
            .iter()
            .enumerate()
            .map(|(i, l)| {
                let content = if i > host {
                    anchor::Content::Group // 読まれない層（host より上）と、画素の要らない祖先のグループ
                } else {
                    match l.kind {
                        LayerKind::Group => anchor::Content::Group,
                        LayerKind::Adjustment => anchor::Content::Adjustment(
                            l.adjustment
                                .clone()
                                .unwrap_or_else(crate::AdjustmentSettings::invert),
                        ),
                        LayerKind::Fill if views[i].is_none() => anchor::Content::Fill(
                            l.fill.get(&channel).copied().unwrap_or(Rgba8::TRANSPARENT),
                        ),
                        _ => anchor::Content::Pixels(views[i].as_ref().unwrap_or(&empty)),
                    }
                };
                let enabled = i <= host
                    && l.is_channel_enabled(channel)
                    && match l.kind {
                        LayerKind::Group => true,
                        LayerKind::Adjustment => {
                            l.adjustment.as_ref().is_some_and(|a| a.applies_to(kind))
                        }
                        LayerKind::Fill => l.fill.contains_key(&channel),
                        LayerKind::Raster => l.surface(channel).is_some(),
                    };
                anchor::Layer {
                    parent: l.parent.and_then(|p| position.get(&p).copied()),
                    content,
                    visible: l.visible,
                    enabled,
                    opacity: l.opacity_in(channel),
                    blend: l.blend_mode_in(channel),
                    clipping: l.clipping,
                    mask: match (&l.mask, &mask_views[i]) {
                        (Some(m), Some(v)) => Some(anchor::Mask {
                            source: v,
                            enabled: m.enabled,
                            inverted: m.inverted,
                            density: m.density,
                        }),
                        _ => None,
                    },
                }
            })
            .collect();
        let plan = anchor::Plan::new(&layers, host, dims, kind).map_err(map_generator_error)?;
        let (x0, y0) = (coord.x * ts, coord.y * ts);
        let region = Rect::new(x0, y0, ts.min(self.width - x0), ts.min(self.height - y0));
        let pixels = plan
            .evaluate(
                region,
                &generator::Options {
                    budget_bytes: u64::from(ts) * u64::from(ts) * 4,
                    cancel,
                },
            )
            .map_err(map_generator_error)?;
        let tile = if pixels.iter().all(|b| *b == 0) {
            None
        } else {
            let mut bytes = vec![0u8; (ts * ts * 4) as usize];
            for r in 0..region.height as usize {
                bytes[r * ts as usize * 4..][..region.width as usize * 4].copy_from_slice(
                    &pixels[r * region.width as usize * 4..][..region.width as usize * 4],
                );
            }
            Some(Arc::new(bytes))
        };
        {
            let mut c = self.cache();
            c.counters.anchor_tiles_composited += 1;
            c.clock += 1;
            let used = c.clock;
            let len = tile.as_ref().map_or(0, |t| t.len() as u64);
            if self.effects.cache_budget > 0 {
                if let Some(old) = c.anchors.remove(&key) {
                    c.anchor_bytes -= old.tile.as_ref().map_or(0, |t| t.len() as u64);
                }
                c.anchor_bytes += len;
                c.anchors.insert(
                    key,
                    AnchorEntry {
                        stamp,
                        tile: tile.clone(),
                        used,
                    },
                );
            }
        }
        self.trim_effect_cache();
        Ok(tile)
    }

    // ───────── Anchor を読む段の解決の見張りと、変化の記録 ─────────

    /// Anchor を読む段の解決（読む Anchor がどの層・置き場か、使えるか）の署名。
    fn anchor_signature(&self) -> Vec<AnchorSig> {
        let points = self.anchor_points();
        let mut v = Vec::new();
        for (i, l) in self.layers.iter().enumerate() {
            let stacks = [
                l.filters.as_slice(),
                l.mask.as_ref().map_or(&[][..], |m| m.filters.as_slice()),
            ];
            for stack in stacks {
                for e in stack {
                    let EffectSettings::Generator(g) = &e.settings else {
                        continue;
                    };
                    if g.kind != generator::Kind::Anchor {
                        continue;
                    }
                    v.push(AnchorSig {
                        reader: l.id,
                        filter: e.id,
                        resolved: g.anchor.resolve(&points, i, self.layers.len()).map(|p| {
                            (
                                self.layers[p.host].id,
                                match p.placement {
                                    anchor::Placement::Layer => AnchorPlacement::Layer,
                                    anchor::Placement::Mask => AnchorPlacement::Mask,
                                },
                            )
                        }),
                        channel: g.anchor.channel,
                        read: g.anchor.read,
                    });
                }
            }
        }
        v
    }

    /// Anchor を読む段がどこかにあるか（有効かは問わない）。
    pub(super) fn has_anchor_readers(&self) -> bool {
        self.layers.iter().any(|l| {
            l.filters
                .iter()
                .chain(l.mask.iter().flat_map(|m| m.filters.iter()))
                .any(|e| e.settings.reads_anchor())
        })
    }

    /// 編集（Undo・Redo を含む）の後に: Anchor を読む段の解決が変わっていれば、読む層を全部変わったことにする
    /// （Anchor を置いた・外した・動かした・参照を選び直した）。値の変化はタイルごとに記録されている。
    pub(super) fn refresh_anchor_readers(&mut self) {
        let now = self.anchor_signature();
        if now == self.effects.anchor_signature {
            return;
        }
        let readers: Vec<LayerId> = now
            .iter()
            .chain(self.effects.anchor_signature.iter())
            .map(|s| s.reader)
            .collect();
        self.effects.anchor_signature = now;
        let mut any = false;
        for id in readers {
            if let Some(i) = self.layer_index(id) {
                any = true;
                self.mark_layer(i, None);
            }
        }
        if any {
            self.journal.serial += 1;
            self.mark_clipped_layers();
        }
    }

    /// 変化したタイルを、Anchor を読む段の出力へ広げる（読む Anchor が変わると、読む層の出力も、段より後のぼかしなどの半径だけ
    /// 広がって変わる）。不動点まで回す。sets はチャンネルの番号ごとの変わったタイル。
    pub(super) fn close_over_anchor_readers(
        &self,
        since: u64,
        sets: &mut HashMap<Channel, std::collections::HashSet<TileCoord>>,
    ) {
        let points = self.anchor_points();
        let (cols, rows) = self.tile_dims();
        let all: Vec<TileCoord> = self.whole_range().iter().collect();
        // (読む層の番号, 影響するチャンネル, 読む元, 段より後の半径)
        struct Reader {
            channels: Vec<Channel>,
            source: Source,
            halo: u32,
            /// 段より後の近傍の段が UV の継ぎ目をまたぐときの帯の幅と段の数（`uv_seams`）。
            seam: (u32, usize),
            /// 段より後に全域の段（正規化など）がある: 読む Anchor の 1 タイルの変化が、読む層の全タイルの出力を変える。
            global: bool,
            /// マスクのスタックの段なら、そのマスクを持つ層（読む元が変わると、そのマスクの出力が変わる）。
            mask_of: Option<LayerId>,
        }
        enum Source {
            Channel(Channel),
            Mask(LayerId),
        }
        let mut readers = Vec::new();
        for (i, l) in self.layers.iter().enumerate() {
            for (target_mask, stack) in [
                (false, l.filters.as_slice()),
                (
                    true,
                    l.mask.as_ref().map_or(&[][..], |m| m.filters.as_slice()),
                ),
            ] {
                let active: Vec<&FilterEffect> = stack.iter().filter(|e| e.is_active()).collect();
                for (k, e) in active.iter().enumerate() {
                    let EffectSettings::Generator(g) = &e.settings else {
                        continue;
                    };
                    if g.kind != generator::Kind::Anchor {
                        continue;
                    }
                    let Ok(p) = g.anchor.resolve(&points, i, self.layers.len()) else {
                        continue;
                    };
                    let halo: u32 = active[k..].iter().map(|e| e.settings.halo()).sum();
                    let seam = self.seam_shape(active[k..].iter().map(|e| e.settings.halo()));
                    let global = active[k..].iter().any(|e| e.settings.is_global());
                    let channels = if target_mask {
                        self.covered_channels(i)
                    } else {
                        e.channels.clone()
                    };
                    readers.push(Reader {
                        channels,
                        source: match p.placement {
                            anchor::Placement::Layer => Source::Channel(g.anchor.channel),
                            anchor::Placement::Mask => Source::Mask(self.layers[p.host].id),
                        },
                        halo,
                        seam,
                        global,
                        mask_of: target_mask.then_some(l.id),
                    });
                }
            }
        }
        // 読む元が変わって出力が変わるマスク（マスクのスタックが Anchor を読む。そのマスクの Anchor を読む層も変わる）
        let mut dirty_masks = std::collections::HashSet::new();
        for _ in 0..=2 * readers.len() {
            let mut grew = false;
            for r in &readers {
                let source: Vec<TileCoord> = match &r.source {
                    Source::Channel(c) => sets
                        .get(c)
                        .map(|s| s.iter().copied().collect())
                        .unwrap_or_default(),
                    Source::Mask(id) => {
                        if self.effects.mask_serials.get(id).copied().unwrap_or(0) > since
                            || dirty_masks.contains(id)
                        {
                            all.clone()
                        } else {
                            Vec::new()
                        }
                    }
                };
                if source.is_empty() {
                    continue;
                }
                if let Some(owner) = r.mask_of {
                    grew |= dirty_masks.insert(owner);
                }
                let m = r.halo.div_ceil(self.tile_size);
                // 継ぎ目をまたいで届く所（全域の段があれば下で全部）
                let across = if r.global {
                    Vec::new()
                } else {
                    self.seam_write_tiles(r.seam, r.halo, source.iter().copied())
                };
                for c in &r.channels {
                    let set = sets.entry(*c).or_default();
                    if r.global {
                        for t in &all {
                            grew |= set.insert(*t);
                        }
                        continue;
                    }
                    for t in &source {
                        for y in t.y.saturating_sub(m)..=(t.y + m).min(rows - 1) {
                            for x in t.x.saturating_sub(m)..=(t.x + m).min(cols - 1) {
                                grew |= set.insert(TileCoord::new(x, y));
                            }
                        }
                    }
                    for t in &across {
                        grew |= set.insert(*t);
                    }
                }
            }
            if !grew {
                break;
            }
        }
    }

    // ───────── 状態の説明 ─────────

    /// Generator の設定が今は使えない（入力のまま通す・値を見せる）理由。使えるなら None。読む層は reader の番号。
    pub(super) fn generator_reason(
        &self,
        g: &generator::Settings,
        reader: usize,
    ) -> Option<InactiveReason> {
        self.generator_status(g, reader).0
    }

    /// [`Self::generator_reason`] と、ノイズ・グランジが位置のマップの代わりに UV で評価している理由（値は出ている）。
    pub(super) fn generator_status(
        &self,
        g: &generator::Settings,
        reader: usize,
    ) -> (Option<InactiveReason>, Option<generator::Inactive>) {
        let inputs = &self.effects.inputs;
        let maps: Vec<generator::Map<'_>> = inputs.maps.iter().map(|m| m.as_generator()).collect();
        let frame = inputs.frame.and_then(|f| f.for_generator().ok());
        let dims = (self.width, self.height);
        let points = self.anchor_points();
        let reference = (g.kind == generator::Kind::Anchor)
            .then(|| g.anchor.resolve(&points, reader, self.layers.len()));
        // 値の読み元は要らない（使えるかの判定だけ）。使える参照には空の読み元を渡す
        struct Nothing(u32, u32);
        impl anchor::ValueSource for Nothing {
            fn dimensions(&self) -> (u32, u32) {
                (self.0, self.1)
            }
            fn value(&self, _: u32, _: u32) -> Option<f64> {
                None
            }
        }
        let nothing = Nothing(dims.0, dims.1);
        let value_source: Result<&dyn anchor::ValueSource, anchor::Issue> = match &reference {
            Some(Err(issue)) => Err(issue.clone()),
            Some(Ok(_)) => Ok(&nothing),
            None => Err(anchor::Issue::NotChosen),
        };
        match BoundGenerator::bind(g, &maps, frame, dims, value_source) {
            // 画像の段: 画像だけを待っているなら、画像を読めるか・投影を組めるかを見る（色かスカラーかで読めるかは変わらない）
            Ok(b) if b.inactive() == Some(&generator::Inactive::MissingImage) => {
                let chain = match self.generator_image_chain(g.image.image, true) {
                    Ok(chain) => chain,
                    Err(why) => return (Some(why), None),
                };
                match self.image_sampler(&g.image.projection, &chain) {
                    Ok(sampler) => (
                        b.with_image(&sampler)
                            .inactive()
                            .cloned()
                            .map(InactiveReason::Generator),
                        None,
                    ),
                    Err(e) => (Some(InactiveReason::Rejected(e.to_string())), None),
                }
            }
            Ok(b) => (
                b.inactive().cloned().map(InactiveReason::Generator),
                b.fallback().cloned(),
            ),
            Err(e) => (Some(InactiveReason::Rejected(e.to_string())), None),
        }
    }

    /// 画像の段の投影のサンプラー（塗りつぶしの層の画像と同じ入力: 文書の大きさ・位置と向きのマップ（最新のものだけ）・モデルのルート）。
    fn image_sampler<'s>(
        &'s self,
        projection: &Projection,
        chain: &'s ImageMipChain<'static>,
    ) -> Result<FillSampler<'s>, fill_image::FillError> {
        let inputs = &self.effects.inputs;
        let position = inputs.map(MapKind::Position);
        let normal = inputs.map(MapKind::WorldNormal);
        FillSampler::bind(FillInput {
            projection: *projection,
            width: self.width,
            height: self.height,
            image: Some(chain),
            shape: None,
            fallback: Rgba8::TRANSPARENT,
            missing_image: false,
            positions: usable_map(position).map(|m| m.as_fill()),
            normals: usable_map(normal).map(|m| m.as_fill()),
            bounds_min: position.map_or([0.; 3], |m| m.bounds_min),
            bounds_max: position.map_or([1.; 3], |m| m.bounds_max),
            frame: inputs.frame.map(|fr| fr.for_fill()),
            stale_position: position.is_some_and(|m| m.state != MapState::Current),
            stale_normal: normal.is_some_and(|m| m.state != MapState::Current),
        })
    }

    /// デカールが今は出ていない理由（位置・法線のマップが使えない・モデルのルートが分からない）。
    pub(super) fn decal_problem(&self, index: usize) -> Option<InactiveReason> {
        let l = &self.layers[index];
        self.projection_problem(&l.projection)
    }

    fn projection_problem(&self, p: &Projection) -> Option<InactiveReason> {
        use generator::Inactive as I;
        if p.mode == ProjectionMode::Uv {
            return None;
        }
        let inputs = &self.effects.inputs;
        let need_normal = matches!(p.mode, ProjectionMode::Triplanar | ProjectionMode::Decal);
        let check = |kind: MapKind| -> Option<InactiveReason> {
            let why = match inputs.map(kind) {
                None => I::MissingMap(kind),
                Some(m) if m.state == MapState::Stale => I::StaleMap(kind),
                Some(m) if m.state != MapState::Current => I::UnverifiedMap(kind),
                Some(m) if (m.width, m.height) != (self.width, self.height) => I::MapSize(kind),
                _ => return None,
            };
            Some(InactiveReason::Generator(why))
        };
        check(MapKind::Position)
            .or_else(|| need_normal.then(|| check(MapKind::WorldNormal)).flatten())
            .or_else(|| {
                inputs
                    .frame
                    .is_none()
                    .then_some(InactiveReason::Generator(I::MissingFrame))
            })
    }

    /// 画像のチャンネルが今は画像を投影していない（値を見せている）理由。
    pub(super) fn fill_image_problem(
        &self,
        index: usize,
        channel: Channel,
        id: ImageId,
    ) -> Option<InactiveReason> {
        let l = &self.layers[index];
        let kind = self.channel_kind(channel).ok()?;
        if let Err(why) = self.mip_chain(id, kind) {
            return Some(why);
        }
        self.projection_problem(&l.projection)
    }
}

/// 評価したタイルの画素。全部同じ色（透明も）なら一様なタイルにする。透明の一様なタイルは「評価したが何も無い」印
/// （C# の評価した出力の、読めたが空のタイル）で、保存する面には置かれない。
fn covered_tile(bytes: Vec<u8>) -> Tile {
    let first = Rgba8::from_slice(&bytes[0..4]);
    if bytes.chunks_exact(4).all(|p| p == first.to_array()) {
        Tile::Uniform(first)
    } else {
        Tile::kept(bytes)
    }
}

fn usable_map(m: Option<&crate::effects::MapInput>) -> Option<&crate::effects::MapInput> {
    m.filter(|m| m.state == MapState::Current)
}

fn collect_layers(plan: &[Entry], out: &mut Vec<usize>) {
    for e in plan {
        out.push(e.layer);
        collect_layers(&e.children, out);
        collect_layers(&e.clips, out);
    }
}

/// 評価した画素（`region` の straight RGBA8、行は下から）の上に、面の画素を「通常」で重ねる（塗りつぶしの層のパス。
/// `src · a + dst · da · (1 − a)` を 8 bit に丸める。パスの作業面へリボンを重ねるのと同じ式）。
fn over_surface(output: &mut [u8], region: Rect, surface: &Surface) -> Result<(), CoreError> {
    let w = region.width as usize;
    let mut row = vec![0u8; w * 4];
    for y in 0..region.height {
        surface.read_row(region.x, region.y + y, &mut row)?;
        let dst = &mut output[y as usize * w * 4..][..w * 4];
        for (d, s) in dst.chunks_exact_mut(4).zip(row.chunks_exact(4)) {
            let a = s[3] as f64 / 255.0;
            if a <= 0.0 {
                continue;
            }
            let da = d[3] as f64 / 255.0;
            let out = a + da * (1.0 - a);
            for k in 0..3 {
                d[k] = ((s[k] as f64 * a + d[k] as f64 * da * (1.0 - a)) / out)
                    .round()
                    .clamp(0.0, 255.0) as u8;
            }
            d[3] = (out * 255.0).round().clamp(0.0, 255.0) as u8;
        }
    }
    Ok(())
}

#[cfg(test)]
mod sync_check {
    fn assert_sync<T: Sync>() {}
    #[test]
    fn document_is_sync() {
        assert_sync::<super::Document>();
    }
}
