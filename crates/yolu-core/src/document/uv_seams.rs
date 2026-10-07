//! レイヤーのフィルターが UV の継ぎ目をまたぐ（文書の設定 [`Document::filter_seams`]。既定は入）。
//!
//! 効果の入力にモデルの UV の位相（`EffectInputs::with_topology`）があり、設定が入のとき、近傍の段（`halo` > 0）のあるスタックは、
//! スタックの近傍の段の半径の最大から決めた帯の幅（`geometry::seam_band_width`）の帯の写しで評価する（`filter::Options::seams`）。
//! ここは、どのスタックがまたぐかと帯の写しを引く所、継ぎ目をまたぐタイルの到達（評価の鍵・タイルの覆い・変化の印）。
//!
//! - 読む側（評価の鍵・覆い）: ブロックの評価が読むタイルの帯のテクセルが読む点のタイルを、前の段の半径の分広げたもの。近傍の段が
//!   いくつもあれば、その数だけ同じことを重ねる（前の段の帯も、またいで読むため）。
//! - 書く側（変化の印）: 元画素のタイルの変化が、半径の分広がった所を読む帯のテクセルのタイルへ届き、そこから半径の分の出力を変える。
//! - 帯の写しはモデル・解像度・帯の幅ごとに `UvTopology` が覚える。初めて要るときに作る（評価・印付けのどちらでも、その場で作って待つ）。

use std::collections::HashSet;
use std::sync::Arc;

use super::eval::SourceKey;
use super::*;
use crate::filter;
use crate::geometry::seam_band::TileDeps;
use crate::geometry::{seam_band_width, SeamBand, UvTopologyError};

/// 継ぎ目をまたぐはずのレイヤーが、またがずに 2D で評価されている理由（[`Document::seam_fallback`]）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SeamFallback {
    /// アイランドの図・帯の写しを、予算（[`Document::seam_cache_budget_bytes`]）の中で作れない。
    Tables(UvTopologyError),
    /// 帯の写しはあるが、使うと 1 ブロックの作業メモリが予算（[`Document::filter_working_budget_bytes`]）を超える。
    Working { needed: u64, budget: u64 },
}

impl Document {
    /// レイヤーのフィルターの近傍の段（ぼかし・シャープなど）が UV の継ぎ目をまたいで読むか（文書の設定。既定は入）。モデルの UV の位相が
    /// 効果の入力に無いときは、入でも今までと同じ（2D の上で読む）。
    pub fn filter_seams(&self) -> bool {
        self.filter_seams
    }

    /// 設定を変える（1 回の Undo。合成が変わるレイヤーに印を付ける）。
    pub fn set_filter_seams(&mut self, on: bool) -> Result<(), CoreError> {
        self.ensure_no_stroke()?;
        if self.filter_seams == on {
            return Ok(());
        }
        self.record(
            Command::FilterSeams {
                old: self.filter_seams,
                new: on,
            },
            16,
            None,
        )
    }

    /// 読み込みで設定を置く（履歴を作らない。読み込みの最中の、まだ何も評価していない文書だけに使う）。
    pub fn set_filter_seams_for_load(&mut self, on: bool) {
        self.filter_seams = on;
    }

    /// 今、レイヤーのフィルターが継ぎ目をまたぐか（設定が入で、モデルの UV の位相が効果の入力にある）。
    pub fn seams_active(&self) -> bool {
        self.filter_seams && self.effects.inputs.topology.is_some()
    }

    pub(super) fn switch_filter_seams(&mut self, on: bool) {
        // 前と後の両方で、またぐ所に印を付ける（切るときは、前にまたいで出していた所も描き直す）
        self.mark_seam_readers();
        self.filter_seams = on;
        self.mark_seam_readers();
    }

    /// 近傍の段のあるレイヤー（内容・マスク）を全部変わったことにする（設定・モデルの UV の位相が変わったとき）。またいでいなければ何もしない。
    pub(super) fn mark_seam_readers(&mut self) {
        if !self.seams_active() {
            return;
        }
        let readers: Vec<usize> = (0..self.layers.len())
            .filter(|&i| {
                let l = &self.layers[i];
                l.filters
                    .iter()
                    .chain(l.mask.iter().flat_map(|m| m.filters.iter()))
                    .any(|e| e.is_active() && e.settings.halo() > 0)
            })
            .collect();
        for &i in &readers {
            self.mark_layer(i, None);
        }
        if !readers.is_empty() {
            self.mark_clipped_layers();
        }
    }

    /// 段の並び（半径の並び）の帯の幅と近傍の段の数（0 はまたがない）。
    pub(super) fn seam_shape(&self, halos: impl Iterator<Item = u32>) -> (u32, usize) {
        if !self.seams_active() {
            return (0, 0);
        }
        let (mut max, mut levels) = (0, 0);
        for h in halos {
            if h > 0 {
                max = max.max(h);
                levels += 1;
            }
        }
        (seam_band_width(max), levels)
    }

    /// 帯の幅 band の、文書の大きさの帯の写し（band が 0・作れないときは None）。
    pub(super) fn seam_table(&self, band: u32) -> Option<Arc<SeamBand>> {
        self.seam_table_at(self.width, self.height, band)
    }

    /// 大きさを選んだ帯の写し（粗い評価の縮めた画像）。
    pub(super) fn seam_table_at(
        &self,
        width: u32,
        height: u32,
        band: u32,
    ) -> Option<Arc<SeamBand>> {
        if band == 0 || !self.filter_seams {
            return None;
        }
        // 覚えてあればそれ。無ければ、アイランドの図・帯の写しの予算の中で作る（収まらなければ None で、またがずに 2D で評価する。
        // 断ったことは `seam_fallback` で分かる）
        let topology = self.effects.inputs.topology.as_ref()?;
        topology
            .seam_band_within(width, height, band, self.effects.seam_budget)
            .ok()
    }

    /// 評価の仕様（スタック）の、文書の大きさの帯の写し。帯の写しが作れない（予算）・使うとブロックの作業メモリが予算を超えるなら None
    /// （またがずに 2D で評価する。どちらも `seam_fallback` で分かる）。
    pub(super) fn seam_table_for(&self, spec: &super::eval::Spec) -> Option<Arc<SeamBand>> {
        let table = self.seam_table(spec.seam.0)?;
        self.seams_fit(&super::eval::spec_stages(spec), &table)
            .then_some(table)
    }

    /// 帯の写し `band` を使って 1 ブロックを評価するときの作業メモリ（ブロックの返す画像を含む。`filter::evaluate` の見積りと同じ式）。
    fn seam_block_need(&self, stages: &[filter::Stage], band: &SeamBand) -> u64 {
        let (w, h) = (band.width(), band.height());
        let block = self.effects.block_pixels;
        let side = (block / self.tile_size).max(1) * self.tile_size;
        let base = filter::block_working_bytes(stages, block, w, h).unwrap_or(u64::MAX);
        let seam = filter::seam_working_bytes(stages, block, w, h, band.texel_count() as u64);
        let output = u64::from(side.min(w)) * u64::from(side.min(h)) * 4;
        base.saturating_add(seam).saturating_add(output)
    }

    /// 帯の写しを使っても、1 ブロックの評価が作業メモリの予算（`filter_working_budget_bytes`）に収まるか。
    pub(super) fn seams_fit(&self, stages: &[filter::Stage], band: &SeamBand) -> bool {
        self.seam_block_need(stages, band) <= self.effects.working_budget
    }

    /// アイランドの図・帯の写しの予算（バイト。既定 256 MiB）。覚えておくアイランドの図と帯の写しの合計と、作る間の作業メモリがこれに収まる。
    /// ブロックの評価の作業メモリの予算（`filter_working_budget_bytes`）とは別。
    pub fn seam_cache_budget_bytes(&self) -> u64 {
        self.effects.seam_budget
    }

    /// アイランドの図・帯の写しの予算を変える（保存しない設定。Undo にもならない）。またげるかが変わり得るので、またぐレイヤーを描き直す。
    pub fn set_seam_cache_budget_bytes(&mut self, bytes: u64) {
        if bytes == self.effects.seam_budget {
            return;
        }
        self.mark_seam_readers();
        self.effects.seam_budget = bytes;
        // 新しい予算に収まらない覚えは捨てる（作り直した文書と同じ結果にする）
        if let Some(topology) = &self.effects.inputs.topology {
            topology.shrink(bytes);
        }
        self.mark_seam_readers();
        // アイランドごとのばらつきも、アイランドの図を作れるかが変わり得る
        self.mark_island_readers();
        // 前の予算で断った・作れた帯の写しの上に評価を重ねない
        self.release_effect_cache();
    }

    /// 継ぎ目をまたぐ設定が入で、モデルの UV の位相もあり、近傍の段のあるレイヤーもあるのに、またがずに 2D で評価している理由。またげている・
    /// またがない・まだ帯の写しを作ろうとしていない（モデルを渡したときと評価のときに作る）ときは None。
    pub fn seam_fallback(&self) -> Option<SeamFallback> {
        if !self.seams_active() {
            return None;
        }
        let topology = self.effects.inputs.topology.as_ref()?;
        // 評価と同じ並び: レイヤーのチャンネルごとの有効な段の並びと、マスクの有効な段の並び
        let mut chains: Vec<Vec<&FilterEffect>> = Vec::new();
        for l in &self.layers {
            chains.extend(Channel::ALL.iter().map(|c| l.active_chain(*c)));
            chains.extend(
                l.mask
                    .as_ref()
                    .map(|m| m.filters.iter().filter(|e| e.is_active()).collect()),
            );
        }
        for chain in chains {
            let (band, _) = self.seam_shape(chain.iter().map(|e| e.settings.halo()));
            if band == 0 {
                continue;
            }
            if let Some(e) = topology.refusal(self.width, self.height, band) {
                return Some(SeamFallback::Tables(e));
            }
            if let Some(table) = topology.cached_seam_band(self.width, self.height, band) {
                let stages = super::effects::stages_of(&chain);
                let needed = self.seam_block_need(&stages, &table);
                if needed > self.effects.working_budget {
                    return Some(SeamFallback::Working {
                        needed,
                        budget: self.effects.working_budget,
                    });
                }
            }
        }
        None
    }

    fn seam_deps(&self, band: u32) -> Option<Arc<TileDeps>> {
        Some(self.seam_table(band)?.deps(self.tile_size))
    }

    /// タイルの組を、半径 halo（画素）の分のタイルへ広げる。
    fn seam_grown(&self, tiles: &HashSet<TileCoord>, halo: u32) -> HashSet<TileCoord> {
        let m = halo.div_ceil(self.tile_size);
        if m == 0 {
            return tiles.clone();
        }
        let cols = self.width.div_ceil(self.tile_size);
        let rows = self.height.div_ceil(self.tile_size);
        let mut out = HashSet::with_capacity(tiles.len() * 4);
        for t in tiles {
            for y in t.y.saturating_sub(m)..=(t.y + m).min(rows - 1) {
                for x in t.x.saturating_sub(m)..=(t.x + m).min(cols - 1) {
                    out.insert(TileCoord::new(x, y));
                }
            }
        }
        out
    }

    /// 読む側の到達: start のタイルの評価（帯の幅 band・近傍の段 levels 個・半径の和 halo）が、継ぎ目をまたいで読む元画素のタイル
    /// （start に無いもの）。
    pub(super) fn seam_read_tiles(
        &self,
        (band, levels): (u32, usize),
        halo: u32,
        start: impl IntoIterator<Item = TileCoord>,
    ) -> Vec<TileCoord> {
        let Some(deps) = self.seam_deps(band) else {
            return Vec::new();
        };
        let start: HashSet<TileCoord> = start.into_iter().collect();
        let mut all: HashSet<TileCoord> = HashSet::new();
        let mut frontier = start.clone();
        for _ in 0..levels {
            let sources: HashSet<TileCoord> =
                frontier.iter().flat_map(|t| deps.sources(*t)).collect();
            let reached = self.seam_grown(&sources, halo);
            let new: HashSet<TileCoord> = reached
                .into_iter()
                .filter(|t| !start.contains(t) && !all.contains(t))
                .collect();
            if new.is_empty() {
                break;
            }
            all.extend(new.iter().copied());
            frontier = new;
        }
        all.into_iter().collect()
    }

    /// 書く側の到達: changed のタイルの元画素の変化が、継ぎ目をまたいで届く出力のタイル（半径の分の広がりは含まない。それは呼ぶ側）。
    pub(super) fn seam_write_tiles(
        &self,
        (band, levels): (u32, usize),
        halo: u32,
        changed: impl IntoIterator<Item = TileCoord>,
    ) -> Vec<TileCoord> {
        let Some(deps) = self.seam_deps(band) else {
            return Vec::new();
        };
        let changed: HashSet<TileCoord> = changed.into_iter().collect();
        let mut all: HashSet<TileCoord> = HashSet::new();
        let mut frontier = self.seam_grown(&changed, halo);
        for _ in 0..levels {
            let readers: HashSet<TileCoord> =
                frontier.iter().flat_map(|t| deps.dependents(*t)).collect();
            let reached = self.seam_grown(&readers, halo);
            let new: HashSet<TileCoord> =
                reached.into_iter().filter(|t| !all.contains(t)).collect();
            if new.is_empty() {
                break;
            }
            all.extend(new.iter().copied());
            frontier = new;
        }
        all.into_iter().collect()
    }

    /// 評価の鍵の元画素の変化の番号に、継ぎ目をまたいで読むタイルの分を足す（読むタイルの最後の変化の番号の最大）。
    pub(super) fn seam_source_serial(
        &self,
        id: LayerId,
        key: SourceKey,
        shape: (u32, usize),
        halo: u32,
        tiles: impl IntoIterator<Item = TileCoord>,
    ) -> u64 {
        if shape.0 == 0 {
            return 0;
        }
        let Some(map) = self.effects.source.get(&(id, key)) else {
            return 0;
        };
        self.seam_read_tiles(shape, halo, tiles)
            .iter()
            .filter_map(|c| map.get(c))
            .copied()
            .max()
            .unwrap_or(0)
    }

    /// タイル coord の出力が、継ぎ目をまたいで読む元画素（surface）の中身を持ち得るか（半径 reach の分を読む）。
    pub(super) fn seam_reaches_content(
        &self,
        shape: (u32, usize),
        reach: u32,
        coord: TileCoord,
        surface: Option<&Surface>,
    ) -> bool {
        let Some(s) = surface else { return false };
        if shape.0 == 0 || reach == 0 || s.tile_count() == 0 {
            return false;
        }
        let start = self.seam_grown(&HashSet::from([coord]), reach);
        self.seam_read_tiles(shape, reach, start)
            .iter()
            .any(|c| s.has_tile(*c))
    }

    /// 元画素のタイル coord の変化を、継ぎ目をまたいで届く出力のタイルへ記録する（チャンネルごと）。
    pub(super) fn mark_seam_reach(
        &mut self,
        channels: &[Channel],
        coord: TileCoord,
        shape: (u32, usize),
        halo: u32,
    ) {
        if shape.0 == 0 {
            return;
        }
        let tiles = self.seam_write_tiles(shape, halo, [coord]);
        for &c in channels {
            for t in &tiles {
                self.journal.mark(c, *t);
            }
        }
    }
}
