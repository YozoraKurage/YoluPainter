//! 文書（C# の PaintDocument・PaintLayer の M1 の部分）。
//!
//! - 層は下から上。どの編集も 1 回の Undo になり、断った編集は何も変えない。
//! - 履歴はタイルの前後の状態そのもの（ブラシの再生や画布全体の写しではない）で、予算（既定 64 MiB）を超えた古い段から落とす。
//! - 進行中のストロークがある間は、ほかの編集・Undo・Redo を断る（ストロークは文書が持ち、[`Stroke`] はその札）。
//! - 変化の記録（`change_serial` と `changed_tiles`）は、合成が変わり得るタイルをチャンネルごとに数で覚える。履歴や保存とは別。

use std::collections::hash_map::RandomState;
use std::collections::HashMap;
use std::fmt;
use std::hash::{BuildHasher, Hasher};

use glam::DVec2;

use crate::brush::{BrushSample, BrushSettings, Budgets, StrokeState};
use crate::composite;
use crate::error::CoreError;
use crate::math::require_finite;
use crate::surface::{Growth, Surface, Tile};
use crate::types::{BlendMode, Channel, Rect, Rgba8, RowOrder, TileCoord};

/// レイヤーの ID（C# の Guid と同じ 128 bit。0 は使わない）。
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LayerId(pub u128);

impl fmt::Debug for LayerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "LayerId({:032x})", self.0)
    }
}
impl fmt::Display for LayerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:032x}", self.0)
    }
}

/// 画素の層（M1 はラスターの層だけ。チャンネルごとに面を持つ）。
#[derive(Clone, Debug)]
pub struct Layer {
    pub(crate) id: LayerId,
    pub(crate) name: String,
    pub(crate) visible: bool,
    pub(crate) opacity: f64,
    pub(crate) blend_mode: BlendMode,
    pub(crate) clipping: bool,
    /// チャンネルの番号ごとの面（無いチャンネルは None）。
    pub(crate) surfaces: [Option<Surface>; 6],
    /// 有効なチャンネルの印（bit = チャンネルの番号）。無効にしたチャンネルの画素は保つ。
    pub(crate) enabled: u8,
}

impl Layer {
    pub fn id(&self) -> LayerId {
        self.id
    }
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn visible(&self) -> bool {
        self.visible
    }
    pub fn opacity(&self) -> f64 {
        self.opacity
    }
    pub fn blend_mode(&self) -> BlendMode {
        self.blend_mode
    }
    /// すぐ下の層（クリッピングの下地）の中にだけ描く印。一番下の層では効かない（印は保つ）。
    pub fn clipping(&self) -> bool {
        self.clipping
    }
    /// チャンネルの面（無ければ None）。
    pub fn surface(&self, channel: Channel) -> Option<&Surface> {
        self.surfaces[channel as usize].as_ref()
    }
    pub fn is_channel_enabled(&self, channel: Channel) -> bool {
        self.enabled & (1 << channel as u8) != 0
    }
    /// 層そのものの画素（不透明度・合成の前）。面が無ければ透明。
    pub fn pixel(&self, channel: Channel, x: u32, y: u32) -> Result<Rgba8, CoreError> {
        match self.surface(channel) {
            Some(s) => s.pixel(x, y),
            None => Ok(Rgba8::TRANSPARENT),
        }
    }
    pub(crate) fn pixel_or_transparent(&self, channel: Channel, x: u32, y: u32) -> Rgba8 {
        self.surface(channel)
            .and_then(|s| s.pixel(x, y).ok())
            .unwrap_or(Rgba8::TRANSPARENT)
    }
    /// 全チャンネルの画素のバイト数。
    pub fn allocated_bytes(&self) -> u64 {
        self.surfaces
            .iter()
            .flatten()
            .map(|s| s.allocated_bytes())
            .sum()
    }
    fn surface_mut(&mut self, channel: Channel) -> Option<&mut Surface> {
        self.surfaces[channel as usize].as_mut()
    }
}

/// ストロークの札。文書を借りないので、フレームをまたいで持てる（中身は文書が持つ）。確定・取消で手放す。
#[derive(Debug)]
pub struct Stroke {
    id: u64,
}

impl Stroke {
    /// ペンの点を足す。x・y は画素の座標（左下原点、画素の中心は n + 0.5）、筆圧は 0〜1、傾きはラジアン（M1 では使わない）。
    /// 時刻は前の点と同じにする（時刻つきは [`Stroke::add_sample`]）。失敗したら、このストロークは取り消してから返す。
    pub fn add_point(
        &mut self,
        doc: &mut Document,
        x: f64,
        y: f64,
        pressure: f64,
        tilt: DVec2,
    ) -> Result<(), CoreError> {
        let time = match &doc.active {
            Some(a) if a.id == self.id => a.last_time(),
            _ => return Err(CoreError::NoActiveStroke),
        };
        let sample = match BrushSample::new(x, y, pressure, time, tilt) {
            Ok(s) => s,
            Err(e) => {
                doc.cancel_active_stroke();
                return Err(e);
            }
        };
        doc.stroke_add(self.id, sample)
    }
    /// 時刻つきの入力（時刻は減ってはならない）。
    pub fn add_sample(&mut self, doc: &mut Document, sample: BrushSample) -> Result<(), CoreError> {
        doc.stroke_add(self.id, sample)
    }
    /// 与えた覆い（0〜1）で 1 画素を塗る（メッシュのダブ。重なる三角形の覆いは呼び手が 1 つにまとめてから）。画布の外は何もしない。
    pub fn apply_pixel(
        &mut self,
        doc: &mut Document,
        x: i64,
        y: i64,
        coverage: f64,
        pressure: f64,
    ) -> Result<bool, CoreError> {
        doc.stroke_apply_pixel(self.id, x, y, coverage, pressure)
    }
    /// この札の番号。
    pub fn id(&self) -> u64 {
        self.id
    }
}

/// ストロークの確定の結果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StrokeResult {
    /// 画素が変わったか（変わらなければ履歴に積まず、Redo も残す）。
    pub changed: bool,
    /// 置いたダブの数。
    pub stamps: u64,
    /// 受け取った入力の点の数。
    pub samples: u64,
}

/// 進行中のストロークの数（確定の前に見る）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StrokeStats {
    pub stamps: u64,
    pub samples: u64,
    /// 手を付けたタイルの数。
    pub tiles: usize,
    /// 巻き戻し用の写しと覆いのバイト数。
    pub rollback_bytes: u64,
    /// 大きなダブのうち、ストロークが持つタイルをワーカーで描いたものの数。
    pub parallel_dabs: u64,
}

#[derive(Clone, Debug)]
pub(crate) struct TileChange {
    coord: TileCoord,
    before: Option<Tile>,
    after: Option<Tile>,
}

#[derive(Clone, Debug)]
enum Property {
    Visible(bool),
    Opacity(f64),
    Mode(BlendMode),
    Clipping(bool),
    Name(String),
}

/// 履歴の 1 段。Insert・Remove の層は、文書に無い側の状態のときに段が持つ。
enum Command {
    Stroke {
        layer: LayerId,
        channel: Channel,
        changes: Vec<TileChange>,
    },
    Insert {
        index: usize,
        layer: Option<Box<Layer>>,
    },
    Remove {
        index: usize,
        layer: Option<Box<Layer>>,
    },
    Move {
        id: LayerId,
        from: usize,
        to: usize,
    },
    Property {
        id: LayerId,
        old: Property,
        new: Property,
    },
}

struct Entry {
    command: Command,
    cost: u64,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum CoalesceKey {
    Opacity(LayerId),
}

/// 変化の記録: チャンネルごとに、タイルが最後に変わった通し番号。
#[derive(Default)]
struct Journal {
    serial: u64,
    tiles: [HashMap<TileCoord, u64>; 6],
}

impl Journal {
    fn mark(&mut self, channel: Channel, coord: TileCoord) {
        self.serial += 1;
        self.tiles[channel as usize].insert(coord, self.serial);
    }
    /// 層の持つタイル全部（全チャンネル）を変わったことにする（C# の MarkLayerChanged）。
    fn mark_layer(&mut self, layer: &Layer) {
        for channel in Channel::ALL {
            if let Some(s) = layer.surface(channel) {
                for coord in s.tile_coords() {
                    self.mark(channel, coord);
                }
            }
        }
    }
}

/// 単独の書き手の CPU の文書。
pub struct Document {
    id: u128,
    width: u32,
    height: u32,
    tile_size: u32,
    layers: Vec<Layer>,
    undo: Vec<Entry>,
    redo: Vec<Entry>,
    history_bytes: u64,
    undo_budget: u64,
    minimum_undo_steps: usize,
    source_budget: u64,
    stroke_budget: u64,
    active: Option<StrokeState>,
    next_stroke: u64,
    revision: u64,
    journal: Journal,
    coalesce: Option<CoalesceKey>,
    trim_count: u64,
    trimmed_bytes: u64,
    id_counter: u64,
}

/// 128 bit の新しい ID（std の RandomState の鍵と通し番号から）。
fn random_id(counter: u64) -> u128 {
    let state = RandomState::new();
    let mut a = state.build_hasher();
    a.write_u64(counter);
    let mut b = state.build_hasher();
    b.write_u64(!counter);
    b.write_u8(0x5a);
    ((a.finish() as u128) << 64) | b.finish() as u128
}

/// UTF-16 の長さ（C# の string.Length。名前の履歴の大きさに使う）。
fn utf16_len(s: &str) -> u64 {
    s.encode_utf16().count() as u64
}

impl Document {
    /// 既定の予算: 画素 256 MiB、ストローク 64 MiB、履歴 64 MiB（C# と同じ）。
    pub const DEFAULT_TILE_SIZE: u32 = 128;

    /// 幅・高さ（1〜32768）の空の文書。タイルは 128。
    pub fn new(width: u32, height: u32) -> Result<Document, CoreError> {
        Self::with_tile_size(width, height, Self::DEFAULT_TILE_SIZE)
    }

    /// タイルの大きさ（1〜1024）を選ぶ。画素の結果はタイルの大きさによらない。
    pub fn with_tile_size(width: u32, height: u32, tile_size: u32) -> Result<Document, CoreError> {
        if width == 0 || width > 32768 {
            return Err(CoreError::InvalidArgument("width"));
        }
        if height == 0 || height > 32768 {
            return Err(CoreError::InvalidArgument("height"));
        }
        if tile_size == 0 || tile_size > 1024 {
            return Err(CoreError::InvalidArgument("tile_size"));
        }
        Ok(Document {
            id: random_id(0) | 1,
            width,
            height,
            tile_size,
            layers: Vec::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            history_bytes: 0,
            undo_budget: 64 * 1024 * 1024,
            minimum_undo_steps: 0,
            source_budget: 256 * 1024 * 1024,
            stroke_budget: 64 * 1024 * 1024,
            active: None,
            next_stroke: 1,
            revision: 0,
            journal: Journal::default(),
            coalesce: None,
            trim_count: 0,
            trimmed_bytes: 0,
            id_counter: 0,
        })
    }

    /// 読み込み直後の文書と層に保存済みのIDを復元する（層は下からの順）。
    /// 呼び出し前のIDを持つ履歴は消す。進行中のストローク・空ID・重複IDは拒否する。
    /// 文書を消費するので、読み込みが完了するまで外部へ公開しないこと。
    pub fn with_persistent_ids(
        mut self,
        document_id: u128,
        layer_ids: &[LayerId],
    ) -> Result<Self, CoreError> {
        self.ensure_no_stroke()?;
        let mut seen = std::collections::HashSet::new();
        if document_id == 0
            || layer_ids.len() != self.layers.len()
            || layer_ids.iter().any(|id| id.0 == 0 || !seen.insert(*id))
        {
            return Err(CoreError::InvalidArgument("persistent_ids"));
        }
        self.id = document_id;
        for (layer, id) in self.layers.iter_mut().zip(layer_ids) {
            layer.id = *id;
        }
        self.external_mutation();
        Ok(self)
    }

    // ───────── 大きさ・状態 ─────────

    /// 文書の ID（作るたびに違う）。
    pub fn id(&self) -> u128 {
        self.id
    }
    pub fn width(&self) -> u32 {
        self.width
    }
    pub fn height(&self) -> u32 {
        self.height
    }
    pub fn tile_size(&self) -> u32 {
        self.tile_size
    }
    /// 画布全体の矩形。
    pub fn bounds(&self) -> Rect {
        Rect::new(0, 0, self.width, self.height)
    }
    /// タイルの矩形（画布の端で切る）。`changed_tiles` のタイルだけを合成し直すときに。画布の外のタイルは None。
    pub fn tile_rect(&self, coord: TileCoord) -> Option<Rect> {
        let ts = self.tile_size as u64;
        let (x, y) = (coord.x as u64 * ts, coord.y as u64 * ts);
        if x >= self.width as u64 || y >= self.height as u64 {
            return None;
        }
        Some(Rect::new(
            x as u32,
            y as u32,
            (self.width as u64 - x).min(ts) as u32,
            (self.height as u64 - y).min(ts) as u32,
        ))
    }
    /// 編集のたびに増える（履歴の段・Undo・Redo・ストロークのダブ）。保存が要るかの目安。
    pub fn revision(&self) -> u64 {
        self.revision
    }
    /// 層の画素の合計のバイト数。
    pub fn allocated_bytes(&self) -> u64 {
        self.layers.iter().map(|l| l.allocated_bytes()).sum()
    }
    pub fn has_active_stroke(&self) -> bool {
        self.active.is_some()
    }
    /// 進行中のストロークの数（無ければ None）。
    pub fn active_stroke_stats(&self) -> Option<StrokeStats> {
        self.active.as_ref().map(|a| StrokeStats {
            stamps: a.stamp_count,
            samples: a.sample_count,
            tiles: a.tiles.len(),
            rollback_bytes: a.rollback_bytes,
            parallel_dabs: a.parallel_dabs,
        })
    }

    // ───────── 層 ─────────

    /// 層（下から上）。
    pub fn layers(&self) -> &[Layer] {
        &self.layers
    }
    pub fn layer(&self, id: LayerId) -> Option<&Layer> {
        self.layers.iter().find(|l| l.id == id)
    }
    pub fn layer_index(&self, id: LayerId) -> Option<usize> {
        self.layers.iter().position(|l| l.id == id)
    }
    fn index_of(&self, id: LayerId) -> Result<usize, CoreError> {
        self.layer_index(id).ok_or(CoreError::LayerNotFound)
    }
    fn ensure_no_stroke(&self) -> Result<(), CoreError> {
        if self.active.is_some() {
            Err(CoreError::StrokeActive)
        } else {
            Ok(())
        }
    }
    fn new_layer_id(&mut self) -> LayerId {
        loop {
            self.id_counter += 1;
            let id = LayerId(random_id(self.id_counter));
            if id.0 != 0 && self.layer(id).is_none() {
                return id;
            }
        }
    }

    /// 空の層を一番上に足す（Color のチャンネルを持つ）。1 回の Undo。
    pub fn add_layer(&mut self, name: &str) -> Result<LayerId, CoreError> {
        self.add_layer_above(name, None)
    }

    /// 空の層を above のすぐ上に足す（None なら一番上）。
    pub fn add_layer_above(
        &mut self,
        name: &str,
        above: Option<LayerId>,
    ) -> Result<LayerId, CoreError> {
        self.ensure_no_stroke()?;
        let index = match above {
            Some(id) => self.index_of(id)? + 1,
            None => self.layers.len(),
        };
        let id = self.new_layer_id();
        let mut surfaces: [Option<Surface>; 6] = Default::default();
        surfaces[Channel::Color as usize] =
            Some(Surface::new(self.width, self.height, self.tile_size));
        let layer = Layer {
            id,
            name: name.to_string(),
            visible: true,
            opacity: 1.0,
            blend_mode: BlendMode::Normal,
            clipping: false,
            surfaces,
            enabled: 1 << Channel::Color as u8,
        };
        self.execute(
            Command::Insert {
                index,
                layer: Some(Box::new(layer)),
            },
            128,
        )?;
        Ok(id)
    }

    /// 層を取り除く。1 回の Undo で、同じ層（ID・画素・属性）が同じ所へ戻る。
    pub fn remove_layer(&mut self, id: LayerId) -> Result<(), CoreError> {
        self.ensure_no_stroke()?;
        let index = self.index_of(id)?;
        let bytes = self.layers[index].allocated_bytes();
        self.execute(Command::Remove { index, layer: None }, 128 + bytes)
    }

    /// 層を並びの new_index（0 = 一番下）へ動かす。同じ位置なら何もしない。
    pub fn move_layer(&mut self, id: LayerId, new_index: usize) -> Result<(), CoreError> {
        self.ensure_no_stroke()?;
        let from = self.index_of(id)?;
        if new_index >= self.layers.len() {
            return Err(CoreError::InvalidArgument("new_index"));
        }
        if new_index == from {
            return Ok(());
        }
        self.execute(
            Command::Move {
                id,
                from,
                to: new_index,
            },
            64,
        )
    }

    pub fn set_layer_visible(&mut self, id: LayerId, visible: bool) -> Result<(), CoreError> {
        self.set_property(id, Property::Visible(visible), None)
    }

    /// 不透明度 0〜1。coalesce なら、間に何も無い同じ層の不透明度の変更へまとめる（スライダーのドラッグ。終わりに `end_coalescing`）。
    pub fn set_layer_opacity(
        &mut self,
        id: LayerId,
        opacity: f64,
        coalesce: bool,
    ) -> Result<(), CoreError> {
        require_finite(opacity, "opacity")?;
        if !(0.0..=1.0).contains(&opacity) {
            return Err(CoreError::InvalidArgument("opacity"));
        }
        self.set_property(
            id,
            Property::Opacity(opacity),
            coalesce.then_some(CoalesceKey::Opacity(id)),
        )
    }

    /// 合成モード（PassThrough はグループだけなので断る）。
    pub fn set_layer_blend_mode(&mut self, id: LayerId, mode: BlendMode) -> Result<(), CoreError> {
        if mode == BlendMode::PassThrough {
            return Err(CoreError::InvalidArgument("PassThrough はグループだけ"));
        }
        self.set_property(id, Property::Mode(mode), None)
    }

    /// すぐ下の層へのクリッピング（とその解除）。
    pub fn set_layer_clipping(&mut self, id: LayerId, clipping: bool) -> Result<(), CoreError> {
        self.set_property(id, Property::Clipping(clipping), None)
    }

    pub fn set_layer_name(&mut self, id: LayerId, name: &str) -> Result<(), CoreError> {
        self.set_property(id, Property::Name(name.to_string()), None)
    }

    /// 印が効いているか: 印があり、下に層がある（一番下の層は何にもクリッピングされない）。
    pub fn is_effectively_clipped(&self, index: usize) -> bool {
        index > 0 && index < self.layers.len() && self.layers[index].clipping
    }

    fn set_property(
        &mut self,
        id: LayerId,
        new: Property,
        key: Option<CoalesceKey>,
    ) -> Result<(), CoreError> {
        self.ensure_no_stroke()?;
        let layer = &self.layers[self.index_of(id)?];
        let old = match &new {
            Property::Visible(_) => Property::Visible(layer.visible),
            Property::Opacity(_) => Property::Opacity(layer.opacity),
            Property::Mode(_) => Property::Mode(layer.blend_mode),
            Property::Clipping(_) => Property::Clipping(layer.clipping),
            Property::Name(_) => Property::Name(layer.name.clone()),
        };
        let same = match (&old, &new) {
            (Property::Visible(a), Property::Visible(b)) => a == b,
            (Property::Opacity(a), Property::Opacity(b)) => a == b,
            (Property::Mode(a), Property::Mode(b)) => a == b,
            (Property::Clipping(a), Property::Clipping(b)) => a == b,
            (Property::Name(a), Property::Name(b)) => a == b,
            _ => false,
        };
        if same {
            return Ok(());
        }
        let cost = match (&old, &new) {
            (Property::Name(a), Property::Name(b)) => 64 + 2 * (utf16_len(a) + utf16_len(b)),
            _ => 64,
        };
        // まとめ: 同じ鍵の直前の段（間に何も無い）へ
        if let Some(k) = key {
            if self.coalesce == Some(k) && self.redo.is_empty() && !self.undo.is_empty() {
                // 直前の段の「後」の値だけを新しくする（戻すと最初の変更の前の値へ）
                self.set_property_value(id, new.clone())?;
                if let Some(Entry {
                    command: Command::Property { new: n, .. },
                    ..
                }) = self.undo.last_mut()
                {
                    *n = new;
                }
                self.revision += 1;
                return Ok(());
            }
        }
        self.execute(Command::Property { id, old, new }, cost)?;
        if let Some(k) = key {
            self.coalesce = Some(k);
        }
        Ok(())
    }

    /// まとめている変更の続きを終える（スライダーのドラッグの終わり）。次の変更は別の Undo になる。
    pub fn end_coalescing(&mut self) {
        self.coalesce = None;
    }

    /// まとめている変更を取り消して、その段ごと捨てる（ドラッグを Escape で止めたとき）。まとめが無ければ false。
    pub fn cancel_coalescing(&mut self) -> Result<bool, CoreError> {
        self.ensure_no_stroke()?;
        if self.coalesce.is_none() || self.undo.is_empty() {
            self.end_coalescing();
            return Ok(false);
        }
        let mut top = self.undo.pop().expect("空でない");
        if let Err(e) = self.revert(&mut top.command) {
            self.undo.push(top);
            return Err(e);
        }
        self.history_bytes -= top.cost;
        self.revision += 1;
        self.end_coalescing();
        Ok(true)
    }

    // ───────── 直接の書き込み（読み込み・管理。履歴を消す） ─────────

    /// 1 タイルを丸ごと読み込む（TileSize² × 4、行優先・下の行から、画布の外の余白は 0）。読み込みなので履歴を消す。
    /// 面の無いチャンネルは作って有効にする。変わったら true。
    pub fn import_tile(
        &mut self,
        id: LayerId,
        channel: Channel,
        coord: TileCoord,
        bytes: &[u8],
    ) -> Result<bool, CoreError> {
        self.ensure_no_stroke()?;
        let index = self.index_of(id)?;
        if channel == Channel::Normal {
            return Err(CoreError::Unsupported("Normal のチャンネルはまだ無い"));
        }
        let growth = self.growth_for(index, channel);
        let (w, h, ts) = (self.width, self.height, self.tile_size);
        let layer = &mut self.layers[index];
        if layer.surfaces[channel as usize].is_none() {
            layer.surfaces[channel as usize] = Some(Surface::new(w, h, ts));
            layer.enabled |= 1 << channel as u8;
        }
        let changed = layer
            .surface_mut(channel)
            .expect("作った")
            .import_tile(coord, bytes, growth)?;
        if changed {
            self.journal.mark(channel, coord);
            self.external_mutation();
        }
        Ok(changed)
    }

    /// 1 画素を直接書く（読み込み・管理用。履歴を消す）。Color の面へ。
    pub fn set_pixel(
        &mut self,
        id: LayerId,
        x: u32,
        y: u32,
        color: Rgba8,
    ) -> Result<bool, CoreError> {
        self.ensure_no_stroke()?;
        let index = self.index_of(id)?;
        let growth = self.growth_for(index, Channel::Color);
        let ts = self.tile_size;
        let surface = self.layers[index]
            .surface_mut(Channel::Color)
            .ok_or(CoreError::Unsupported("Color の面が無い"))?;
        let changed = surface.set_pixel(x, y, color, growth)?;
        if changed {
            self.journal
                .mark(Channel::Color, TileCoord::new(x / ts, y / ts));
            self.external_mutation();
        }
        Ok(changed)
    }

    fn external_mutation(&mut self) {
        self.clear_history_unchecked();
        self.revision += 1;
    }

    /// 履歴（Undo・Redo）を消す。
    pub fn clear_history(&mut self) -> Result<(), CoreError> {
        self.ensure_no_stroke()?;
        self.clear_history_unchecked();
        Ok(())
    }
    fn clear_history_unchecked(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.history_bytes = 0;
        self.coalesce = None;
    }

    // ───────── 予算 ─────────

    /// 層の画素の予算（既定 256 MiB）。今の画素より小さくはできない。
    pub fn source_budget_bytes(&self) -> u64 {
        self.source_budget
    }
    pub fn set_source_budget_bytes(&mut self, bytes: u64) -> Result<(), CoreError> {
        self.ensure_no_stroke()?;
        if bytes < self.allocated_bytes() {
            return Err(CoreError::InvalidArgument("予算が今の画素より小さい"));
        }
        self.source_budget = bytes;
        Ok(())
    }
    /// 進行中のストロークの巻き戻し用の写しと覆いの予算（既定 64 MiB）。
    pub fn stroke_budget_bytes(&self) -> u64 {
        self.stroke_budget
    }
    pub fn set_stroke_budget_bytes(&mut self, bytes: u64) -> Result<(), CoreError> {
        self.ensure_no_stroke()?;
        self.stroke_budget = bytes;
        Ok(())
    }
    /// 履歴の予算（既定 64 MiB）。超えた古い段から落とす（`minimum_undo_steps` の新しい段は超えても残す）。
    pub fn undo_budget_bytes(&self) -> u64 {
        self.undo_budget
    }
    pub fn set_undo_budget_bytes(&mut self, bytes: u64) -> Result<(), CoreError> {
        self.ensure_no_stroke()?;
        self.undo_budget = bytes;
        self.trim_history();
        Ok(())
    }
    pub fn minimum_undo_steps(&self) -> usize {
        self.minimum_undo_steps
    }
    pub fn set_minimum_undo_steps(&mut self, steps: usize) -> Result<(), CoreError> {
        self.ensure_no_stroke()?;
        self.minimum_undo_steps = steps;
        self.trim_history();
        Ok(())
    }
    /// 履歴のバイト数（タイルの写しと段の名目の大きさ）。
    pub fn history_bytes(&self) -> u64 {
        self.history_bytes
    }
    /// 予算で古い段を落とした回数と、落としたバイト数の合計（C# の HistoryTrimming の通知の代わり）。
    pub fn history_trimmed(&self) -> (u64, u64) {
        (self.trim_count, self.trimmed_bytes)
    }
    fn growth_for(&self, index: usize, channel: Channel) -> Growth {
        let own = self.layers[index]
            .surface(channel)
            .map_or(0, |s| s.allocated_bytes());
        Growth {
            budget: self.source_budget,
            others: self.allocated_bytes() - own,
        }
    }

    // ───────── 履歴 ─────────

    pub fn undo_count(&self) -> usize {
        self.undo.len()
    }
    pub fn redo_count(&self) -> usize {
        self.redo.len()
    }
    pub fn can_undo(&self) -> bool {
        self.active.is_none() && !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        self.active.is_none() && !self.redo.is_empty()
    }

    /// 1 段戻す。戻す段が無ければ false。進行中のストロークがあれば断る。
    pub fn undo(&mut self) -> Result<bool, CoreError> {
        self.ensure_no_stroke()?;
        let Some(mut entry) = self.undo.pop() else {
            return Ok(false);
        };
        self.end_coalescing();
        if let Err(e) = self.revert(&mut entry.command) {
            self.undo.push(entry);
            return Err(e);
        }
        self.redo.push(entry);
        self.revision += 1;
        Ok(true)
    }

    /// 1 段やり直す。
    pub fn redo(&mut self) -> Result<bool, CoreError> {
        self.ensure_no_stroke()?;
        let Some(mut entry) = self.redo.pop() else {
            return Ok(false);
        };
        self.end_coalescing();
        if let Err(e) = self.apply(&mut entry.command) {
            self.redo.push(entry);
            return Err(e);
        }
        self.undo.push(entry);
        self.revision += 1;
        Ok(true)
    }

    /// 段を当てて履歴へ積む（C# の Execute）。
    fn execute(&mut self, mut command: Command, cost: u64) -> Result<(), CoreError> {
        self.apply(&mut command)?;
        self.revision += 1;
        self.push(Entry { command, cost });
        Ok(())
    }

    fn push(&mut self, entry: Entry) {
        self.coalesce = None; // 新しい段はまとめを終える
        for old in self.redo.drain(..) {
            self.history_bytes -= old.cost;
        }
        self.history_bytes += entry.cost;
        self.undo.push(entry);
        self.trim_history();
    }

    /// 予算を超えたら、遠い段（古い Undo、次に遠い Redo）から落とす。新しい minimum_undo_steps 段は超えても残す。
    fn trim_history(&mut self) {
        if self.history_bytes <= self.undo_budget {
            return;
        }
        let mut projected = self.history_bytes;
        let mut discarded = 0;
        let droppable = self.undo.len().saturating_sub(self.minimum_undo_steps);
        for e in self.undo.iter().take(droppable) {
            if projected <= self.undo_budget {
                break;
            }
            projected -= e.cost;
            discarded += e.cost;
        }
        for e in &self.redo {
            if projected <= self.undo_budget {
                break;
            }
            projected -= e.cost;
            discarded += e.cost;
        }
        if discarded == 0 {
            return; // 守る段だけが超えている
        }
        self.trim_count += 1;
        self.trimmed_bytes += discarded;
        while self.history_bytes > self.undo_budget && self.undo.len() > self.minimum_undo_steps {
            let e = self.undo.remove(0);
            self.history_bytes -= e.cost;
        }
        while self.history_bytes > self.undo_budget && !self.redo.is_empty() {
            let e = self.redo.remove(0);
            self.history_bytes -= e.cost;
        }
    }

    /// 段を当てる（やり直し・最初の実行）。
    fn apply(&mut self, command: &mut Command) -> Result<(), CoreError> {
        match command {
            Command::Stroke {
                layer,
                channel,
                changes,
            } => self.restore_tiles(*layer, *channel, changes, false),
            Command::Insert { index, layer } => {
                let l = layer.take().expect("段が層を持つ");
                self.ensure_source_growth(l.allocated_bytes())?;
                self.layers.insert(*index, *l);
                self.journal.mark_layer(&self.layers[*index]);
                self.mark_clipped_layers();
                Ok(())
            }
            Command::Remove { index, layer } => {
                self.journal.mark_layer(&self.layers[*index]);
                *layer = Some(Box::new(self.layers.remove(*index)));
                self.mark_clipped_layers();
                Ok(())
            }
            Command::Move { id, from, to } => {
                let l = self.layers.remove(*from);
                debug_assert_eq!(l.id, *id);
                self.layers.insert(*to, l);
                self.journal.mark_layer(&self.layers[*to]);
                self.mark_clipped_layers();
                Ok(())
            }
            Command::Property { id, new, .. } => self.set_property_value(*id, new.clone()),
        }
    }

    /// 段を戻す。
    fn revert(&mut self, command: &mut Command) -> Result<(), CoreError> {
        match command {
            Command::Stroke {
                layer,
                channel,
                changes,
            } => self.restore_tiles(*layer, *channel, changes, true),
            Command::Insert { index, layer } => {
                let l = self.layers.remove(*index);
                self.journal.mark_layer(&l);
                *layer = Some(Box::new(l));
                self.mark_clipped_layers();
                Ok(())
            }
            Command::Remove { index, layer } => {
                let l = layer.take().expect("段が層を持つ");
                if let Err(e) = self.ensure_source_growth(l.allocated_bytes()) {
                    *layer = Some(l);
                    return Err(e);
                }
                self.layers.insert(*index, *l);
                self.journal.mark_layer(&self.layers[*index]);
                self.mark_clipped_layers();
                Ok(())
            }
            Command::Move { id, from, to } => {
                let l = self.layers.remove(*to);
                debug_assert_eq!(l.id, *id);
                self.layers.insert(*from, l);
                self.journal.mark_layer(&self.layers[*from]);
                self.mark_clipped_layers();
                Ok(())
            }
            Command::Property { id, old, .. } => self.set_property_value(*id, old.clone()),
        }
    }

    fn set_property_value(&mut self, id: LayerId, value: Property) -> Result<(), CoreError> {
        let index = self.index_of(id)?;
        let layer = &mut self.layers[index];
        let marks = !matches!(value, Property::Name(_)); // 名前は合成を変えない
        match value {
            Property::Visible(v) => layer.visible = v,
            Property::Opacity(v) => layer.opacity = v,
            Property::Mode(v) => layer.blend_mode = v,
            Property::Clipping(v) => layer.clipping = v,
            Property::Name(v) => layer.name = v,
        }
        if marks {
            self.journal.mark_layer(&self.layers[index]);
            self.mark_clipped_layers();
        }
        Ok(())
    }

    /// クリッピングの印のある層（一番下も）を全部変わったことにする: 並べ替え・表示などで下地が変わると、見える所が変わる。
    fn mark_clipped_layers(&mut self) {
        for l in &self.layers {
            if l.clipping {
                self.journal.mark_layer(l);
            }
        }
    }

    fn ensure_source_growth(&self, additional: u64) -> Result<(), CoreError> {
        Growth {
            budget: self.source_budget,
            others: self.allocated_bytes(),
        }
        .ensure(0, additional as i64)
    }

    /// ストロークの段の前後のタイルを置く（予算はまとめて先に確かめるので、断ったら何も変えない）。
    fn restore_tiles(
        &mut self,
        layer: LayerId,
        channel: Channel,
        changes: &[TileChange],
        backwards: bool,
    ) -> Result<(), CoreError> {
        let index = self.index_of(layer)?;
        let growth = self.growth_for(index, channel);
        let surface = self.layers[index]
            .surface_mut(channel)
            .ok_or(CoreError::Unsupported("面が無い"))?;
        let delta: i64 = changes
            .iter()
            .map(|c| {
                surface.growth_to(
                    c.coord,
                    if backwards {
                        c.before.as_ref()
                    } else {
                        c.after.as_ref()
                    },
                )
            })
            .sum();
        growth.ensure(surface.allocated_bytes(), delta)?;
        for c in changes {
            surface.restore(
                c.coord,
                if backwards {
                    c.before.as_ref()
                } else {
                    c.after.as_ref()
                },
            );
        }
        for c in changes {
            self.journal.mark(channel, c.coord);
        }
        Ok(())
    }

    // ───────── ストローク ─────────

    /// Color のチャンネルへのストロークを始める。設定はここで写して固定する。
    pub fn begin_stroke(
        &mut self,
        layer: LayerId,
        brush: &BrushSettings,
    ) -> Result<Stroke, CoreError> {
        self.begin_stroke_in(layer, Channel::Color, brush)
    }

    /// チャンネルを選んでストロークを始める（面が無ければ作って有効にする。無効のチャンネルは断る）。
    pub fn begin_stroke_in(
        &mut self,
        layer: LayerId,
        channel: Channel,
        brush: &BrushSettings,
    ) -> Result<Stroke, CoreError> {
        self.ensure_no_stroke()?;
        brush.validate()?;
        if channel == Channel::Normal {
            return Err(CoreError::Unsupported("Normal のチャンネルはまだ無い"));
        }
        let index = self.index_of(layer)?;
        let (w, h, ts) = (self.width, self.height, self.tile_size);
        let l = &mut self.layers[index];
        if l.surfaces[channel as usize].is_none() {
            l.surfaces[channel as usize] = Some(Surface::new(w, h, ts));
            l.enabled |= 1 << channel as u8;
        }
        if !l.is_channel_enabled(channel) {
            return Err(CoreError::Unsupported("無効のチャンネルには描けない"));
        }
        let budgets = Budgets {
            growth: self.growth_for(index, channel),
            stroke: self.stroke_budget,
        };
        let id = self.next_stroke;
        self.next_stroke += 1;
        self.active = Some(StrokeState::new(id, layer, index, channel, *brush, budgets));
        Ok(Stroke { id })
    }

    fn stroke_add(&mut self, id: u64, sample: BrushSample) -> Result<(), CoreError> {
        self.with_stroke(id, |state, surface, changed| {
            state.add(surface, sample, changed)
        })
        .map(|_| ())
    }

    fn stroke_apply_pixel(
        &mut self,
        id: u64,
        x: i64,
        y: i64,
        coverage: f64,
        pressure: f64,
    ) -> Result<bool, CoreError> {
        self.with_stroke(id, |state, surface, changed| {
            state.apply_pixel(surface, x, y, coverage, pressure, changed)
        })
    }

    /// 進行中のストロークと面へ f を当て、変わったタイルを記録する。失敗したらストロークを取り消してから返す。
    fn with_stroke<F>(&mut self, id: u64, f: F) -> Result<bool, CoreError>
    where
        F: FnOnce(&mut StrokeState, &mut Surface, &mut Vec<TileCoord>) -> Result<bool, CoreError>,
    {
        let state = match self.active.as_mut() {
            Some(a) if a.id == id => a,
            _ => return Err(CoreError::NoActiveStroke),
        };
        let channel = state.channel;
        let surface = self.layers[state.layer_index].surfaces[channel as usize]
            .as_mut()
            .expect("ストロークの面");
        let mut changed = Vec::new();
        let result = f(state, surface, &mut changed);
        for coord in changed {
            self.journal.mark(channel, coord);
        }
        match result {
            Ok(any) => {
                if any {
                    self.revision += 1;
                }
                Ok(any)
            }
            Err(e) => {
                self.cancel_active_stroke();
                Err(e)
            }
        }
    }

    /// ストロークを確定する: 変わったタイルの前後を 1 回の Undo として積む。何も変わらなければ積まない（Redo も残す）。
    pub fn end_stroke(&mut self, stroke: Stroke) -> Result<StrokeResult, CoreError> {
        match &self.active {
            Some(a) if a.id == stroke.id => {}
            _ => return Err(CoreError::NoActiveStroke),
        }
        let state = self.active.take().expect("確かめた");
        let surface = self.layers[state.layer_index].surfaces[state.channel as usize]
            .as_mut()
            .expect("ストロークの面");
        let mut coords: Vec<TileCoord> = state.tiles.keys().copied().collect();
        coords.sort();
        let mut changes = Vec::new();
        for coord in coords {
            surface.compact(coord);
            let after = surface.tile(coord).cloned();
            let before = state.tiles[&coord].before.clone();
            if !Tile::same(before.as_ref(), after.as_ref()) {
                changes.push(TileChange {
                    coord,
                    before,
                    after,
                });
            }
        }
        let result = StrokeResult {
            changed: !changes.is_empty(),
            stamps: state.stamp_count,
            samples: state.sample_count,
        };
        if !changes.is_empty() {
            let cost = 64
                + changes
                    .iter()
                    .map(|c| {
                        16 + c.before.as_ref().map_or(0, |t| t.byte_size())
                            + c.after.as_ref().map_or(0, |t| t.byte_size())
                    })
                    .sum::<u64>();
            self.push(Entry {
                command: Command::Stroke {
                    layer: state.layer,
                    channel: state.channel,
                    changes,
                },
                cost,
            });
        }
        Ok(result)
    }

    /// ストロークを取り消す（描く前の画素へ戻す）。札がもう終わっていれば何もしない。
    pub fn cancel_stroke(&mut self, stroke: Stroke) {
        if matches!(&self.active, Some(a) if a.id == stroke.id) {
            self.cancel_active_stroke();
        }
    }

    /// 進行中のストロークを（札が無くても）取り消す。フォーカスを失った・Escape・札を落としたときに。取り消したら true。
    pub fn cancel_active_stroke(&mut self) -> bool {
        let Some(state) = self.active.take() else {
            return false;
        };
        let surface = self.layers[state.layer_index].surfaces[state.channel as usize]
            .as_mut()
            .expect("ストロークの面");
        let mut restored = false;
        let mut coords: Vec<TileCoord> = state.tiles.keys().copied().collect();
        coords.sort();
        for coord in &coords {
            surface.restore(*coord, state.tiles[coord].before.as_ref());
            restored = true;
        }
        for coord in coords {
            self.journal.mark(state.channel, coord);
        }
        if restored {
            self.revision += 1;
        }
        true
    }

    // ───────── 合成 ─────────

    /// Color の合成（straight RGBA8、行は下から上）。
    pub fn composite(&self, rect: Rect) -> Result<Vec<u8>, CoreError> {
        let mut out = vec![0u8; rect.width as usize * rect.height as usize * 4];
        self.composite_into(Channel::Color, rect, &mut out, RowOrder::BottomUp)?;
        Ok(out)
    }

    /// 合成を呼び手の領域（width × height × 4）へ。order で行の並びを選ぶ。描いている途中でも読める。
    pub fn composite_into(
        &self,
        channel: Channel,
        rect: Rect,
        out: &mut [u8],
        order: RowOrder,
    ) -> Result<(), CoreError> {
        self.check_rect(rect)?;
        if channel == Channel::Normal {
            return Err(CoreError::Unsupported(
                "Normal のチャンネルの合成はまだ無い",
            ));
        }
        if out.len() != rect.width as usize * rect.height as usize * 4 {
            return Err(CoreError::InvalidArgument("出力の大きさが矩形と違う"));
        }
        composite::composite_into(&self.layers, self.tile_size, channel, rect, out, order);
        Ok(())
    }

    /// 1 画素の合成（参照の式。画素ごとに層を引くので遅い。試験・スポイト向け）。
    pub fn composite_pixel(&self, channel: Channel, x: u32, y: u32) -> Result<Rgba8, CoreError> {
        if x >= self.width || y >= self.height {
            return Err(CoreError::InvalidArgument("画素が画布の外"));
        }
        if channel == Channel::Normal {
            return Err(CoreError::Unsupported(
                "Normal のチャンネルの合成はまだ無い",
            ));
        }
        Ok(composite::composite_pixel(&self.layers, channel, x, y))
    }

    fn check_rect(&self, rect: Rect) -> Result<(), CoreError> {
        if rect.x as u64 + rect.width as u64 > self.width as u64
            || rect.y as u64 + rect.height as u64 > self.height as u64
        {
            Err(CoreError::InvalidArgument("矩形が画布の外"))
        } else {
            Ok(())
        }
    }

    // ───────── 変化の記録 ─────────

    /// 変化の通し番号。合成を読んだ後にこれを覚え、次に `changed_tiles` へ渡す。
    pub fn change_serial(&self) -> u64 {
        self.journal.serial
    }

    /// since（`change_serial` の値）の後に合成が変わり得るタイル（Y、次に X の順）。画素の変化（ストローク・取消・Undo・Redo・
    /// 読み込み）と、層の並び・表示・不透明度・モード・クリッピング・追加・削除のときはその層の持つタイル全部。
    /// 元に戻った所を含むことがある。since がこの文書の番号でなければ None（全部を描き直す）。
    pub fn changed_tiles(&self, channel: Channel, since: u64) -> Option<Vec<TileCoord>> {
        if since > self.journal.serial {
            return None;
        }
        let mut v: Vec<TileCoord> = self.journal.tiles[channel as usize]
            .iter()
            .filter(|(_, &s)| s > since)
            .map(|(c, _)| *c)
            .collect();
        v.sort();
        Some(v)
    }
}
