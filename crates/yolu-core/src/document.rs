//! 文書（C# の PaintDocument）。
//!
//! - 層は下から上の平らな並び。グループの中身はグループのすぐ下に続けて並ぶ（[`Layer::parent`]）。
//! - どの編集も 1 回の Undo になり、断った編集は何も変えない。
//! - 履歴はタイルの前後の状態そのもの（ブラシの再生や画布全体の写しではない）と、属性・構造の前後で、予算（既定 64 MiB）を
//!   超えた古い段から落とす。
//! - 進行中のストロークがある間は、ほかの編集・Undo・Redo を断る（ストロークは文書が持ち、[`Stroke`] はその札）。
//! - 変化の記録（`change_serial` と `changed_tiles`）は、合成が変わり得るタイルをチャンネルごとに数で覚える。履歴や保存とは別。
//! - チャンネルは文書の一覧（[`ChannelInfo`]）。0〜5 は標準の 6 つで、ユーザーチャンネルは足せる（[`Document::add_channel`]）。

mod batch;
mod history;
pub use history::HistoryKind;
mod clipboard;
mod clone_source;
mod edits;
mod effects;
mod eval;
mod layer_path;
pub(crate) mod locks;
mod material;
mod merge;
mod operations;
mod regions;
mod resize;
mod transform;
mod warp;
pub use warp::{Homography, LiquifyDab, LiquifyMode, Warp, WarpMesh, WarpPoint};
mod triangle_fill;
pub use clipboard::{ClipboardRefusal, ClipboardSource, PasteResult, PixelClipboard};
pub use locks::LayerLocks;
pub use merge::{LayerMergeReport, MergeMethod, MergeRefusal};
pub use resize::{CanvasResampling, PreparedResize, ResizeReport};
pub use transform::{Affine2D, Resampling};
pub use triangle_fill::TriangleFill;
mod selection;
mod smart;
mod snapshot;
mod smart_resample;
mod structure;

use std::collections::hash_map::RandomState;
use std::collections::HashMap;
use std::hash::{BuildHasher, Hasher};

use glam::DVec2;

use crate::adjust::AdjustmentSettings;
use crate::brush::{
    Brush, BrushMappedPixel, BrushPixel, BrushSample, BrushSettings, Budgets, StencilPoint,
    StrokeState,
};
use crate::composite::{self, Stack};
use crate::effects::{Anchor, AnchorPlacement, FilterEffect, FilterId, FilterTarget};
use crate::error::CoreError;
use crate::fill_image::Projection;
use crate::layer::{ChannelBlend, RasterMask};
use crate::math::require_finite;
use crate::normal::NormalSettings;
use crate::selection::SelectionMask;
use crate::surface::{Growth, Surface, Tile};
use crate::types::{
    BlendMode, Channel, ChannelInfo, ChannelKind, LayerKind, Rect, Rgba8, RowOrder, TileCoord,
};

pub use crate::layer::{Layer, LayerId};
pub use eval::EffectCounters;
pub(crate) use eval::EvalSet;

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
    /// このストロークが使うブラシ（始めたときに写して固定したもの）。3D の面のダブが、大きさと硬さの筆圧をストロークと同じ応え
    /// （最小値と曲線）で決めるのに使う。終わったストロークは断る。
    pub fn brush(&self, doc: &Document) -> Result<std::sync::Arc<Brush>, CoreError> {
        match &doc.active {
            Some(a) if a.id == self.id => Ok(a.brush().clone()),
            _ => Err(CoreError::NoActiveStroke),
        }
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
    /// 面のダブを丸ごと塗る（3D の面のブラシ。C# の ApplyDab）: ダブの中で重なりをまとめた画素と被覆率を、渡した順に塗る。
    /// 効果のブラシ（ぼかし・指先・クローン）は、どの画素を書くより前に読み元を凍結する。center は指先の中心（画布の画素の座標）。
    /// 画布の外の画素は飛ばす。失敗したら、このストロークは取り消してから返す。
    pub fn apply_dab(
        &mut self,
        doc: &mut Document,
        pixels: &[BrushPixel],
        center: DVec2,
        pressure: f64,
    ) -> Result<bool, CoreError> {
        doc.with_stroke(self.id, |state, surface, changed| {
            state.apply_dab(surface, pixels, center, pressure, None, changed)
        })
    }
    /// 面のダブを、画素ごとのステンシルの上の点（pixels と同じ数・同じ順）で塗る（3D のビューが模型の上の点を画面へ写して渡す）。
    /// ステンシルの無いストロークでは点は使わない。
    pub fn apply_dab_at(
        &mut self,
        doc: &mut Document,
        pixels: &[BrushPixel],
        center: DVec2,
        pressure: f64,
        points: &[StencilPoint],
    ) -> Result<bool, CoreError> {
        doc.with_stroke(self.id, |state, surface, changed| {
            state.apply_dab(surface, pixels, center, pressure, Some(points), changed)
        })
    }
    /// 1 画素を、ステンシルの上の点 at で読んで塗る（[`Stroke::apply_pixel`] のステンシルの点つき）。
    pub fn apply_pixel_at(
        &mut self,
        doc: &mut Document,
        x: i64,
        y: i64,
        coverage: f64,
        pressure: f64,
        at: StencilPoint,
    ) -> Result<bool, CoreError> {
        doc.with_stroke(self.id, |state, surface, changed| {
            state.apply_pixel(surface, x, y, coverage, pressure, Some(at), changed)
        })
    }
    /// クローンが読む元を、描く層ではなく見えている層の重なり（チャンネルごとの合成）にする（C# の UseCompositeCloneSource）。
    /// 最初のダブの前に、層が画素を持ち得るタイルだけを合成して凍結する（書き込みの途中で合成を読み返さない）。複数チャンネルの
    /// ストロークは、チャンネルごとにそのチャンネルの合成を凍結する。タイルの写しと索引はストロークの予算に数える。
    /// クローン以外・マスクへのストローク・最初のダブより後・予算を超える、のどれも、このストロークを取り消してから返す。
    pub fn use_composite_clone_source(&mut self, doc: &mut Document) -> Result<(), CoreError> {
        doc.use_composite_clone_source(self.id)
    }
    /// 写像された面のダブ（3D の面のクローン・指先。C# の ApplyMappedDab）: 画素ごとに、読む場所（画素中心の最大 4 点と重み。UV の島の
    /// 継ぎ目の向こうも）を呼び手が決めて渡す。全ての読みをどの画素を書くより前に済ませるので、同じダブの中で先に変えた画素を読まない。
    /// 指先の最初の拾いと面の方向は呼び手が決める。points はステンシルの画素ごとの点（pixels と同じ数）、sampling_bytes は呼び手の
    /// 参照の計画（展開の図）の名目のバイトで、ダブの間だけ予算に数える。同じ画素の重複・範囲外・参照の無い画素・クローンでも指先でも
    /// ないブラシ・予算超過は、このストロークを取り消してから返す。
    pub fn apply_mapped_dab(
        &mut self,
        doc: &mut Document,
        pixels: &[BrushMappedPixel],
        pressure: f64,
        sampling_bytes: u64,
        points: Option<&[StencilPoint]>,
    ) -> Result<bool, CoreError> {
        let targets = 1 + doc.material.extra.len();
        doc.with_stroke(self.id, |state, surface, changed| {
            state.apply_mapped_dab(
                surface,
                pixels,
                pressure,
                sampling_bytes,
                points,
                targets,
                changed,
            )
        })
    }
    /// 指先の前の位置を忘れる（3D の面で UV の継ぎ目をまたぐとき。次のダブは位置を覚えるだけ）。
    pub fn reset_effect_direction(&mut self, doc: &mut Document) -> Result<(), CoreError> {
        match doc.active.as_mut() {
            Some(a) if a.id == self.id => {
                a.reset_effect_direction();
                for state in &mut doc.material.extra {
                    state.reset_effect_direction();
                }
                Ok(())
            }
            _ => Err(CoreError::NoActiveStroke),
        }
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
    /// 同じ入力を受けるチャンネル（面）の数（複数チャンネルのストロークは 2 以上）。
    pub targets: usize,
}

/// ストロークが描く面: 層のチャンネルか、層のマスク。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Target {
    Channel(Channel),
    Mask,
}

#[derive(Clone, Debug)]
pub(crate) struct TileChange {
    coord: TileCoord,
    before: Option<Tile>,
    after: Option<Tile>,
}

/// 層の属性（Undo の前後の値）。
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Property {
    Locks(LayerLocks),
    Visible(bool),
    Opacity(f64),
    Mode(BlendMode),
    Clipping(bool),
    Name(String),
    MaskEnabled(bool),
    MaskInverted(bool),
    MaskDensity(f64),
    Adjustment(AdjustmentSettings),
    ChannelBlend(Channel, ChannelBlend),
}

/// 層の並びと入れ子（下から上の ID と親）。
type Order = Vec<(LayerId, Option<LayerId>)>;

/// 履歴の 1 段。Insert・Remove・Structure の層は、文書に無い側の状態のときに段が持つ。
pub(crate) enum Command {
    Material(material::MaterialCommand),
    IdColors {
        old: crate::mesh_maps::IdColorAssignments,
        new: crate::mesh_maps::IdColorAssignments,
    },
    Swap(Box<operations::State>),
    Stroke {
        layer: LayerId,
        target: Target,
        changes: Vec<TileChange>,
    },
    /// まとまり（層、またはグループと中身。len 枚）を index へ入れる。
    Insert {
        index: usize,
        len: usize,
        block: Option<Vec<Layer>>,
    },
    /// まとまりを index から抜く。
    Remove {
        index: usize,
        len: usize,
        block: Option<Vec<Layer>>,
    },
    /// 並び・入れ子の切り替え（移動・グループ化・解除）。spare は文書に無い側の層（グループの層）。
    Structure {
        before: Order,
        after: Order,
        moved: Vec<LayerId>,
        spare: Vec<Layer>,
    },
    Property {
        id: LayerId,
        old: Property,
        new: Property,
    },
    /// チャンネルの有効・無効。有効にして初めて面ができたときは、取り消しで（空の）面を外し、やり直しで作り直す
    /// （ストロークの段は面を層とチャンネルで引くので、作り直した面へ戻せる）。
    ChannelEnabled {
        id: LayerId,
        channel: Channel,
        enabled: bool,
        had_surface: bool,
    },
    /// 塗りつぶしの値（値を置くとチャンネルを有効にする。取り消しで元の有効に戻す）。now_enabled は当てた後の有効
    /// （まとめたドラッグでは最後の変更の後のもの。途中で値を置いて有効にした分も残る）。
    FillValue {
        id: LayerId,
        channel: Channel,
        old: Option<Rgba8>,
        new: Option<Rgba8>,
        was_enabled: bool,
        now_enabled: bool,
    },
    /// マスクを足す（apply で付け、revert で外して持つ）。
    AddMask {
        id: LayerId,
        mask: Option<Box<RasterMask>>,
    },
    /// マスクを外す（apply で外して持ち、revert で付け直す）。
    RemoveMask {
        id: LayerId,
        mask: Option<Box<RasterMask>>,
    },
    SwapSmartMask {
        id: LayerId,
        mask: Option<Box<RasterMask>>,
    },
    NormalSettings {
        old: NormalSettings,
        new: NormalSettings,
    },
    /// チャンネルの一覧の 1 つを変える（足す・消す・変える）。消すときは層のそのチャンネルの中身を段が持つ。
    ChannelInfo {
        channel: Channel,
        old: Option<ChannelInfo>,
        new: Option<ChannelInfo>,
        contents: Vec<(LayerId, structure::ChannelContents)>,
    },
    /// 選択範囲の置き換え（画素は変えない）。
    Selection {
        old: Option<SelectionMask>,
        new: Option<SelectionMask>,
    },
    /// 層（内容）またはマスクのフィルターのスタックの入れ替え。
    Stack {
        id: LayerId,
        target: FilterTarget,
        before: Vec<FilterEffect>,
        after: Vec<FilterEffect>,
    },
    /// Anchor を置く・外す・名前を変える。
    Anchor {
        id: LayerId,
        placement: AnchorPlacement,
        old: Option<Anchor>,
        new: Option<Anchor>,
    },
    /// 塗りつぶしのチャンネルの値・有効・画像・グラデーションの組。
    FillChannel {
        id: LayerId,
        channel: Channel,
        old: Box<effects::FillChannelState>,
        new: Box<effects::FillChannelState>,
    },
    /// 塗りつぶしの投影。
    Projection {
        id: LayerId,
        old: Projection,
        new: Projection,
    },
    /// 層のパスを付ける・差し替える・外す（画素の入れ替えを伴うことがある）。
    Path(layer_path::PathCommand),
    /// 複数の段を 1 段にまとめたもの（`Document::batch`・貼り付け）。当てるのは先頭から、戻すのは末尾から。
    Compound(Vec<Entry>),
}

pub(crate) struct Entry {
    kind: HistoryKind,
    command: Command,
    cost: u64,
}

/// まとめる（スライダーのドラッグを 1 回の Undo にする）変更の鍵。
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum CoalesceKey {
    Opacity(LayerId),
    MaskDensity(LayerId),
    Adjustment(LayerId),
    Fill(LayerId, Channel),
    ChannelBlend(LayerId, Channel),
    NormalSettings,
    FilterStrength(FilterId),
    FilterSettings(FilterId),
    Projection(LayerId),
    FillGradient(LayerId, Channel),
}

/// 変化の記録: チャンネルごとに、タイルが最後に変わった通し番号。
#[derive(Default)]
struct Journal {
    serial: u64,
    tiles: Vec<HashMap<TileCoord, u64>>,
}

impl Journal {
    fn mark(&mut self, channel: Channel, coord: TileCoord) {
        self.serial += 1;
        let i = channel.index();
        if self.tiles.len() <= i {
            self.tiles.resize_with(i + 1, HashMap::new);
        }
        self.tiles[i].insert(coord, self.serial);
    }
}

/// 層の画素の予算の既定（256 MiB。新しい文書と、予算を指定しない読み込みの値）。
pub const DEFAULT_SOURCE_BUDGET_BYTES: u64 = 256 * 1024 * 1024;

/// 単独の書き手の CPU の文書。
pub struct Document {
    id: u128,
    width: u32,
    height: u32,
    tile_size: u32,
    layers: Vec<Layer>,
    /// チャンネルの一覧（番号ごと。None は空き）。0〜5 は標準で必ずある。
    channels: Vec<Option<ChannelInfo>>,
    pub(crate) normal_settings: NormalSettings,
    /// 今の選択範囲（None は選択なし）。
    selection: Option<SelectionMask>,
    undo: Vec<Entry>,
    redo: Vec<Entry>,
    history_bytes: u64,
    undo_budget: u64,
    minimum_undo_steps: usize,
    source_budget: u64,
    stroke_budget: u64,
    active: Option<StrokeState>,
    material: material::MaterialState,
    triangle_fill: Option<triangle_fill::TriangleState>,
    id_colors: crate::mesh_maps::IdColorAssignments,
    /// 進行中のストロークが描く面（active があるときだけ意味がある）。
    active_target: Target,
    next_stroke: u64,
    revision: u64,
    journal: Journal,
    coalesce: Option<CoalesceKey>,
    trim_count: u64,
    trimmed_bytes: u64,
    id_counter: u64,
    /// 効果（フィルター・Generator・Anchor・画像・グラデーション）の評価と、外から渡す入力。保存も Undo もしない。
    effects: eval::EffectState,
    /// `Document::batch` の編集を実行している間 true（ストローク・Undo・Redo・履歴を消す書き込みを断る）。
    batching: bool,
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
            channels: Channel::ALL
                .iter()
                .map(|c| ChannelInfo::standard(*c))
                .collect(),
            normal_settings: NormalSettings::DEFAULT,
            selection: None,
            undo: Vec::new(),
            redo: Vec::new(),
            history_bytes: 0,
            undo_budget: 64 * 1024 * 1024,
            minimum_undo_steps: 0,
            source_budget: DEFAULT_SOURCE_BUDGET_BYTES,
            stroke_budget: 64 * 1024 * 1024,
            active: None,
            material: material::MaterialState::default(),
            triangle_fill: None,
            id_colors: crate::mesh_maps::IdColorAssignments::default(),
            active_target: Target::Channel(Channel::Color),
            next_stroke: 1,
            revision: 0,
            journal: Journal::default(),
            coalesce: None,
            trim_count: 0,
            trimmed_bytes: 0,
            id_counter: 0,
            effects: eval::EffectState::default(),
            batching: false,
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
        self.ensure_loadable()?;
        let mut seen = std::collections::HashSet::new();
        if document_id == 0
            || layer_ids.len() != self.layers.len()
            || layer_ids.iter().any(|id| id.0 == 0 || !seen.insert(*id))
        {
            return Err(CoreError::InvalidArgument("persistent_ids"));
        }
        self.id = document_id;
        // グループの中の層の親も新しい ID へ（並びは同じなので、古い ID → 新しい ID の表で引く）
        let map: HashMap<LayerId, LayerId> = self
            .layers
            .iter()
            .zip(layer_ids)
            .map(|(l, id)| (l.id, *id))
            .collect();
        for (layer, id) in self.layers.iter_mut().zip(layer_ids) {
            layer.id = *id;
            layer.parent = layer.parent.map(|p| map[&p]);
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
    /// 画布のタイルの座標全部（Y、次に X）。
    pub fn canvas_tiles(&self) -> impl Iterator<Item = TileCoord> {
        let (cols, rows) = (
            self.width.div_ceil(self.tile_size),
            self.height.div_ceil(self.tile_size),
        );
        (0..rows).flat_map(move |y| (0..cols).map(move |x| TileCoord::new(x, y)))
    }
    /// 編集のたびに増える（履歴の段・Undo・Redo・ストロークのダブ）。保存が要るかの目安。
    pub fn revision(&self) -> u64 {
        self.revision
    }
    /// 層の画素（チャンネルの面とマスク）の合計のバイト数。
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
            tiles: self.triangle_fill.as_ref().map_or_else(
                || {
                    a.tiles.len()
                        + self
                            .material
                            .extra
                            .iter()
                            .map(|s| s.tiles.len())
                            .sum::<usize>()
                },
                |s| s.tile_count(),
            ),
            rollback_bytes: self.triangle_fill.as_ref().map_or_else(
                || {
                    a.rollback_total()
                        + self
                            .material
                            .extra
                            .iter()
                            .map(|s| s.rollback_total())
                            .sum::<u64>()
                },
                |s| s.rollback,
            ),
            parallel_dabs: a.parallel_dabs
                + self
                    .material
                    .extra
                    .iter()
                    .map(|s| s.parallel_dabs)
                    .sum::<u64>(),
            targets: 1 + self.material.extra.len(),
        })
    }

    // ───────── チャンネルの一覧 ─────────

    /// 文書のチャンネル（番号の順。標準の 6 つと、足したユーザーチャンネル）。
    pub fn channels(&self) -> Vec<Channel> {
        self.channels
            .iter()
            .enumerate()
            .filter(|(_, c)| c.is_some())
            .map(|(i, _)| Channel::from_index(i).expect("64 未満"))
            .collect()
    }
    /// チャンネルの情報（文書に無ければ None）。
    pub fn channel_info(&self, channel: Channel) -> Option<&ChannelInfo> {
        self.channels.get(channel.index()).and_then(|c| c.as_ref())
    }
    pub(crate) fn require_channel(&self, channel: Channel) -> Result<&ChannelInfo, CoreError> {
        self.channel_info(channel).ok_or(CoreError::ChannelNotFound)
    }
    pub(crate) fn channel_kind(&self, channel: Channel) -> Result<ChannelKind, CoreError> {
        Ok(self.require_channel(channel)?.kind)
    }

    // ───────── 層 ─────────

    /// 層（下から上。グループの中身はグループのすぐ下）。
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
    /// ストローク・Undo・Redo・履歴を消す書き込み（読み込み用の直接の書き込み）は、`batch` の編集の中では断る
    /// （まとめた段の位置と、巻き戻せることを守る）。
    fn ensure_not_batching(&self) -> Result<(), CoreError> {
        if self.batching {
            Err(CoreError::BatchActive)
        } else {
            Ok(())
        }
    }
    /// ストロークのない・まとめの中でもない（読み込み用の直接の書き込みの入口）。
    fn ensure_loadable(&self) -> Result<(), CoreError> {
        self.ensure_no_stroke()?;
        self.ensure_not_batching()
    }
    /// まとめている最中か（`batch` の編集の中）。
    pub fn is_batching(&self) -> bool {
        self.batching
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

    /// 合成モード（PassThrough はグループだけ）。
    pub fn set_layer_blend_mode(&mut self, id: LayerId, mode: BlendMode) -> Result<(), CoreError> {
        if mode == BlendMode::PassThrough && !self.layer(id).is_some_and(|l| l.is_group()) {
            self.index_of(id)?;
            return Err(CoreError::InvalidArgument("PassThrough はグループだけ"));
        }
        self.set_property(id, Property::Mode(mode), None)
    }

    /// すぐ下の兄弟へのクリッピング（とその解除）。
    pub fn set_layer_clipping(&mut self, id: LayerId, clipping: bool) -> Result<(), CoreError> {
        self.set_property(id, Property::Clipping(clipping), None)
    }

    pub fn set_layer_name(&mut self, id: LayerId, name: &str) -> Result<(), CoreError> {
        self.set_property(id, Property::Name(name.to_string()), None)
    }

    /// 印が効いているか: 印があり、同じグループの中で下に兄弟がある（兄弟の一番下は何にもクリッピングされない）。
    pub fn is_effectively_clipped(&self, index: usize) -> bool {
        if index == 0 || index >= self.layers.len() || !self.layers[index].clipping {
            return false;
        }
        let parent = self.layers[index].parent;
        self.layers[..index].iter().any(|l| l.parent == parent)
    }

    /// 属性の変更を 1 段として記録する（同じ値なら何もしない）。
    fn set_property(
        &mut self,
        id: LayerId,
        new: Property,
        key: Option<CoalesceKey>,
    ) -> Result<(), CoreError> {
        self.ensure_no_stroke()?;
        let layer = &self.layers[self.index_of(id)?];
        // マスクの設定はマスクの有無がロックより先（C# の RequireMask のあとに RefuseLockedAttributes）
        if matches!(
            new,
            Property::MaskEnabled(_) | Property::MaskInverted(_) | Property::MaskDensity(_)
        ) {
            Self::mask_of(layer)?;
        }
        if !matches!(
            new,
            Property::Visible(_) | Property::Name(_) | Property::Locks(_)
        ) {
            self.refuse_lock(id, LayerLocks::ALL)?;
        }
        let old = match &new {
            Property::Locks(_) => Property::Locks(layer.locks),
            Property::Visible(_) => Property::Visible(layer.visible),
            Property::Opacity(_) => Property::Opacity(layer.opacity),
            Property::Mode(_) => Property::Mode(layer.blend_mode),
            Property::Clipping(_) => Property::Clipping(layer.clipping),
            Property::Name(_) => Property::Name(layer.name.clone()),
            Property::MaskEnabled(_) => Property::MaskEnabled(Self::mask_of(layer)?.enabled),
            Property::MaskInverted(_) => Property::MaskInverted(Self::mask_of(layer)?.inverted),
            Property::MaskDensity(_) => Property::MaskDensity(Self::mask_of(layer)?.density),
            Property::Adjustment(_) => Property::Adjustment(
                layer
                    .adjustment
                    .as_ref()
                    .ok_or(CoreError::Unsupported("調整の層だけが調整の設定を持つ"))?
                    .clone(),
            ),
            Property::ChannelBlend(c, _) => Property::ChannelBlend(*c, layer.channel_blend(*c)),
        };
        if old == new {
            return Ok(());
        }
        let cost = match (&old, &new) {
            (Property::Name(a), Property::Name(b)) => 64 + 2 * (utf16_len(a) + utf16_len(b)),
            (Property::Adjustment(a), Property::Adjustment(b)) => a.byte_size() + b.byte_size(),
            (Property::Adjustment(_), _) => 128,
            _ => 64,
        };
        self.record(Command::Property { id, old, new }, cost, key)
    }

    fn mask_of(layer: &Layer) -> Result<&RasterMask, CoreError> {
        layer
            .mask
            .as_ref()
            .ok_or(CoreError::Unsupported("層にマスクが無い"))
    }

    /// 段を当てて積む。鍵があり、直前の段が同じ鍵のまとめなら、その段の「後」の値だけを新しくする（戻すと最初の変更の前へ）。
    fn record(
        &mut self,
        mut command: Command,
        cost: u64,
        key: Option<CoalesceKey>,
    ) -> Result<(), CoreError> {
        if let Some(k) = key {
            if self.coalesce == Some(k) && self.redo.is_empty() && !self.undo.is_empty() {
                self.apply(&mut command)?;
                let top = &mut self.undo.last_mut().expect("空でない").command;
                match (top, command) {
                    (Command::Property { new: n, .. }, Command::Property { new, .. }) => *n = new,
                    (
                        Command::FillValue {
                            new: n,
                            now_enabled: e,
                            ..
                        },
                        Command::FillValue {
                            new, now_enabled, ..
                        },
                    ) => {
                        *n = new;
                        *e = now_enabled;
                    }
                    (
                        Command::NormalSettings { new: n, .. },
                        Command::NormalSettings { new, .. },
                    ) => *n = new,
                    (Command::Stack { after: n, .. }, Command::Stack { after, .. }) => *n = after,
                    (Command::Projection { new: n, .. }, Command::Projection { new, .. }) => {
                        *n = new
                    }
                    (Command::FillChannel { new: n, .. }, Command::FillChannel { new, .. }) => {
                        *n = new
                    }
                    _ => unreachable!("まとめる段は同じ種類"),
                }
                self.revision += 1;
                return Ok(());
            }
        }
        self.execute(command, cost)?;
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
        self.ensure_loadable()?;
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

    /// 直前の段を取り消し、やり直しにも残さず捨てる（呼び手が積んだ段を、あとの失敗で無かったことにするため）。ただし、積んだ段が押し
    /// 出した古い Undo の段と、消えた Redo は戻らない。複数の文書の大きさを変えるときは、先に `prepare_resize_image` で全部を準備して
    /// から入れれば、これを使わずに済む。`undo` と違い、戻した段を `redo` に積まないので、
    /// 利用者のやり直しで取り消した操作が復活しない。戻す段が無ければ false。
    pub fn discard_last_step(&mut self) -> Result<bool, CoreError> {
        self.ensure_no_stroke()?;
        let Some(mut entry) = self.undo.pop() else {
            return Ok(false);
        };
        self.end_coalescing();
        if let Err(e) = self.revert(&mut entry.command) {
            self.undo.push(entry);
            return Err(e);
        }
        self.history_bytes -= entry.cost;
        self.revision += 1;
        Ok(true)
    }

    // ───────── 直接の書き込み（読み込み・管理。履歴を消す） ─────────

    /// 1 タイルを丸ごと読み込む（TileSize² × 4、行優先・下の行から、画布の外の余白は 0）。読み込みなので履歴を消す。
    /// ラスターの層だけ。面の無いチャンネルは作って有効にする。変わったら true。
    pub fn import_tile(
        &mut self,
        id: LayerId,
        channel: Channel,
        coord: TileCoord,
        bytes: &[u8],
    ) -> Result<bool, CoreError> {
        self.ensure_loadable()?;
        self.require_channel(channel)?;
        let index = self.index_of(id)?;
        self.ensure_raster(index)?;
        let growth = self.growth_for(index, Target::Channel(channel));
        let created = self.ensure_surface(index, channel);
        let changed = match self.layers[index]
            .surface_mut(channel)
            .expect("作った")
            .import_tile(coord, bytes, growth)
        {
            Ok(c) => c,
            Err(e) => {
                self.drop_created_surface(index, channel, created);
                return Err(e);
            }
        };
        if changed {
            self.mark_target_tile(index, Target::Channel(channel), coord);
            self.external_mutation();
        }
        Ok(changed)
    }

    /// マスクの 1 タイルを丸ごと読み込む（隠す量はアルファ、RGB は 0 でなければ断る）。読み込みなので履歴を消す。
    pub fn import_mask_tile(
        &mut self,
        id: LayerId,
        coord: TileCoord,
        bytes: &[u8],
    ) -> Result<bool, CoreError> {
        self.ensure_loadable()?;
        let index = self.index_of(id)?;
        if self.layers[index].mask.is_none() {
            return Err(CoreError::Unsupported("層にマスクが無い"));
        }
        if bytes
            .chunks_exact(4)
            .any(|p| p[0] != 0 || p[1] != 0 || p[2] != 0)
        {
            return Err(CoreError::InvalidArgument("マスクの画素の RGB は 0"));
        }
        let growth = self.growth_for(index, Target::Mask);
        let changed = self.layers[index]
            .mask
            .as_mut()
            .expect("確かめた")
            .surface
            .import_tile(coord, bytes, growth)?;
        if changed {
            self.mark_target_tile(index, Target::Mask, coord);
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
        self.set_channel_pixel(id, Channel::Color, x, y, color)
    }

    /// 1 画素を直接書く（ラスターの層のチャンネルへ。面が無ければ作って有効にする。履歴を消す）。
    pub fn set_channel_pixel(
        &mut self,
        id: LayerId,
        channel: Channel,
        x: u32,
        y: u32,
        color: Rgba8,
    ) -> Result<bool, CoreError> {
        self.ensure_loadable()?;
        self.require_channel(channel)?;
        let index = self.index_of(id)?;
        self.ensure_raster(index)?;
        let growth = self.growth_for(index, Target::Channel(channel));
        let created = self.ensure_surface(index, channel);
        let ts = self.tile_size;
        let surface = self.layers[index].surface_mut(channel).expect("作った");
        let changed = match surface.set_pixel(x, y, color, growth) {
            Ok(c) => c,
            Err(e) => {
                self.drop_created_surface(index, channel, created);
                return Err(e);
            }
        };
        if changed {
            self.mark_target_tile(
                index,
                Target::Channel(channel),
                TileCoord::new(x / ts, y / ts),
            );
            self.external_mutation();
        }
        Ok(changed)
    }

    /// マスクの 1 画素の隠す量を直接書く（履歴を消す）。
    pub fn set_mask_pixel(
        &mut self,
        id: LayerId,
        x: u32,
        y: u32,
        hide: u8,
    ) -> Result<bool, CoreError> {
        self.ensure_loadable()?;
        let index = self.index_of(id)?;
        if self.layers[index].mask.is_none() {
            return Err(CoreError::Unsupported("層にマスクが無い"));
        }
        let growth = self.growth_for(index, Target::Mask);
        let ts = self.tile_size;
        let surface = &mut self.layers[index].mask.as_mut().expect("確かめた").surface;
        let changed = surface.set_pixel(x, y, Rgba8::new(0, 0, 0, hide), growth)?;
        if changed {
            self.mark_target_tile(index, Target::Mask, TileCoord::new(x / ts, y / ts));
            self.external_mutation();
        }
        Ok(changed)
    }

    fn ensure_raster(&self, index: usize) -> Result<(), CoreError> {
        match self.layers[index].kind {
            LayerKind::Raster => Ok(()),
            LayerKind::Fill => Err(CoreError::Unsupported("塗りつぶしの層には描けない")),
            LayerKind::Adjustment => Err(CoreError::Unsupported("調整の層には描けない")),
            LayerKind::Group => Err(CoreError::Unsupported("グループには描けない")),
        }
    }

    /// ラスターの層の面が無ければ作って有効にする（C# の GetChannel。履歴には入らない）。作ったら true。
    fn ensure_surface(&mut self, index: usize, channel: Channel) -> bool {
        let (w, h, ts) = (self.width, self.height, self.tile_size);
        let layer = &mut self.layers[index];
        if layer.surface(channel).is_some() {
            return false;
        }
        layer.put_surface(channel, Some(Surface::new(w, h, ts)));
        layer.set_enabled(channel, true);
        // 空の面でもそのチャンネルで層が合成に入る（クリッピングの組が増えれば、下地のグループが分離になる）
        if layer.clipping {
            self.mark_clip_bases();
        }
        true
    }

    /// ensure_surface で作った面を、断ったときに外す（作る前と同じに）。
    fn drop_created_surface(&mut self, index: usize, channel: Channel, created: bool) {
        if created {
            let layer = &mut self.layers[index];
            layer.put_surface(channel, None);
            layer.set_enabled(channel, false);
            if layer.clipping {
                self.mark_clip_bases();
            }
        }
    }

    fn external_mutation(&mut self) {
        self.clear_history_unchecked();
        self.revision += 1;
        // 直接の書き込み（読み込み・管理）は、層の元画素の変化の記録を通らないことがある: 評価済みのものは全部作り直す
        self.effects.generation += 1;
        self.release_effect_cache();
        self.refresh_anchor_readers();
    }

    /// 履歴（Undo・Redo）を消す。
    pub fn clear_history(&mut self) -> Result<(), CoreError> {
        self.ensure_loadable()?;
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

    /// 層の画素の予算（既定は `DEFAULT_SOURCE_BUDGET_BYTES`）。今の画素より小さくはできない。
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
    fn growth_for(&self, index: usize, target: Target) -> Growth {
        let own = self
            .target_surface(index, target)
            .map_or(0, |s| s.allocated_bytes());
        Growth {
            budget: self.source_budget,
            others: self.allocated_bytes() - own,
        }
    }
    fn target_surface(&self, index: usize, target: Target) -> Option<&Surface> {
        match target {
            Target::Channel(c) => self.layers[index].surface(c),
            Target::Mask => self.layers[index].mask.as_ref().map(|m| &m.surface),
        }
    }
    fn target_surface_mut(&mut self, index: usize, target: Target) -> Option<&mut Surface> {
        match target {
            Target::Channel(c) => self.layers[index].surface_mut(c),
            Target::Mask => self.layers[index].mask.as_mut().map(|m| &mut m.surface),
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
        self.ensure_loadable()?;
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
        self.ensure_loadable()?;
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
        let kind = command.history_kind();
        self.apply(&mut command)?;
        self.revision += 1;
        self.push(Entry {
            command,
            cost,
            kind,
        });
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
        // まとめの途中で古い段を落とすと、失敗したときに元へ戻せない（`batch` が終わってから整理する）
        if self.batching || self.history_bytes <= self.undo_budget {
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
        self.switch(command, false)
    }

    /// 段を戻す。
    fn revert(&mut self, command: &mut Command) -> Result<(), CoreError> {
        self.switch(command, true)
    }

    /// 段を当てる（backwards なら戻す）。断ったら何も変えない（予算はまとめて先に確かめる）。当てた後で、Anchor を読む段の解決が
    /// 変わっていれば読む層を全部変わったことにする。
    fn switch(&mut self, command: &mut Command, backwards: bool) -> Result<(), CoreError> {
        let result = self.switch_command(command, backwards);
        if result.is_ok()
            && !matches!(
                command,
                Command::Stroke { .. } | Command::NormalSettings { .. } | Command::Selection { .. }
            )
        {
            self.refresh_anchor_readers();
        }
        result
    }

    fn switch_command(&mut self, command: &mut Command, backwards: bool) -> Result<(), CoreError> {
        if !matches!(
            command,
            Command::Stroke { .. } | Command::NormalSettings { .. } | Command::Selection { .. }
        ) {
            // クリッピングの組が変わると、下地のグループが通過と分離を行き来する: 変わる前の下地にも印を
            self.mark_clip_bases();
        }
        match command {
            Command::Material(m) => self.restore_material(m, backwards),
            Command::IdColors { old, new } => {
                self.id_colors = if backwards { old.clone() } else { new.clone() };
                Ok(())
            }
            Command::Swap(state) => self.swap_state(state),
            Command::Stroke {
                layer,
                target,
                changes,
            } => self.restore_tiles(*layer, *target, changes, backwards),
            Command::Insert { index, len, block } => {
                if backwards {
                    self.take_block(*index, *len, block)
                } else {
                    self.put_block(*index, block)
                }
            }
            Command::Remove { index, len, block } => {
                if backwards {
                    self.put_block(*index, block)
                } else {
                    self.take_block(*index, *len, block)
                }
            }
            Command::Structure {
                before,
                after,
                moved,
                spare,
            } => {
                let to = if backwards { before } else { after };
                // 段が持っていた層（グループの層とマスク）が文書へ戻るなら、増える分を先に確かめる
                let incoming: u64 = spare
                    .iter()
                    .filter(|l| to.iter().any(|e| e.0 == l.id))
                    .map(|l| l.allocated_bytes())
                    .sum();
                let outgoing: u64 = self
                    .layers
                    .iter()
                    .filter(|l| !to.iter().any(|e| e.0 == l.id))
                    .map(|l| l.allocated_bytes())
                    .sum();
                if incoming > outgoing {
                    self.ensure_source_growth(incoming - outgoing)?;
                }
                self.restore_structure(to, spare);
                for id in moved.iter() {
                    self.mark_layer_anywhere(*id, spare, None);
                }
                self.mark_clipped_layers();
                Ok(())
            }
            Command::Property { id, old, new } => {
                self.set_property_value(*id, if backwards { old.clone() } else { new.clone() })
            }
            Command::ChannelEnabled {
                id,
                channel,
                enabled,
                had_surface,
            } => self.switch_channel_enabled(*id, *channel, *enabled, *had_surface, backwards),
            Command::FillValue {
                id,
                channel,
                old,
                new,
                was_enabled,
                now_enabled,
            } => {
                let index = self.index_of(*id)?;
                let layer = &mut self.layers[index];
                let (value, enabled) = if backwards {
                    (old, was_enabled)
                } else {
                    (new, now_enabled)
                };
                match value {
                    Some(v) => layer.fill.insert(*channel, *v),
                    None => layer.fill.remove(channel),
                };
                layer.set_enabled(*channel, *enabled);
                self.mark_layer(index, Some(*channel));
                self.mark_clipped_layers();
                Ok(())
            }
            Command::SwapSmartMask { id, mask } => {
                let i = self.index_of(*id)?;
                let old = self.layers[i]
                    .mask
                    .as_ref()
                    .map_or(0, |m| m.surface.allocated_bytes());
                let next = mask.as_ref().map_or(0, |m| m.surface.allocated_bytes());
                self.ensure_source_growth(next.saturating_sub(old))?;
                let previous = self.layers[i].mask.take();
                self.layers[i].mask = mask.take().map(|m| *m);
                *mask = previous.map(Box::new);
                // 入れ替わったマスクの画素は、評価のキャッシュの鍵（元画素の通し番号）の外で変わる: 同じ ID・設定のフィルターを持つマスクへ
                // 画素だけが替わっても、古い結果を返さないよう、どちらのマスクのタイルも変わったことにする
                let coords: std::collections::BTreeSet<TileCoord> =
                    [self.layers[i].mask.as_ref(), mask.as_deref()]
                        .into_iter()
                        .flatten()
                        .flat_map(|m| m.surface.tile_coords())
                        .collect();
                for coord in coords {
                    self.note_source(i, Target::Mask, coord);
                }
                self.mark_layer(i, None);
                self.mark_clipped_layers();
                Ok(())
            }
            Command::AddMask { id, mask } => self.switch_mask(*id, mask, !backwards),
            Command::RemoveMask { id, mask } => self.switch_mask(*id, mask, backwards),
            Command::NormalSettings { old, new } => {
                // 合成は変えない（出力だけ）ので、タイルの変化は記録しない
                self.normal_settings = if backwards { *old } else { *new };
                Ok(())
            }
            Command::ChannelInfo {
                channel,
                old,
                new,
                contents,
            } => self.switch_channel_info(*channel, old, new, contents, backwards),
            Command::Selection { old, new } => {
                // 合成は変えないので、タイルの変化は記録しない
                self.selection = if backwards { old.clone() } else { new.clone() };
                Ok(())
            }
            Command::Stack {
                id,
                target,
                before,
                after,
            } => self.switch_stack(*id, *target, if backwards { before } else { after }),
            Command::Anchor {
                id,
                placement,
                old,
                new,
            } => self.switch_anchor(
                *id,
                *placement,
                if backwards {
                    old.as_ref()
                } else {
                    new.as_ref()
                },
            ),
            Command::FillChannel {
                id,
                channel,
                old,
                new,
            } => self.switch_fill_channel(*id, *channel, if backwards { old } else { new }),
            Command::Projection { id, old, new } => {
                self.switch_projection(*id, if backwards { *old } else { *new })
            }
            Command::Path(m) => self.switch_path(m, backwards),
            Command::Compound(steps) => self.switch_compound(steps, backwards),
        }
    }

    /// マスクを付ける（attach）か外す。段が持つマスクを文書と入れ替える。
    fn switch_mask(
        &mut self,
        id: LayerId,
        held: &mut Option<Box<RasterMask>>,
        attach: bool,
    ) -> Result<(), CoreError> {
        let index = self.index_of(id)?;
        if attach {
            let m = held.take().expect("段がマスクを持つ");
            if let Err(e) = self.ensure_source_growth(m.surface.allocated_bytes()) {
                *held = Some(m);
                return Err(e);
            }
            self.layers[index].mask = Some(*m);
        } else {
            *held = Some(Box::new(
                self.layers[index].mask.take().expect("層がマスクを持つ"),
            ));
        }
        self.mark_layer(index, None);
        self.mark_clipped_layers();
        Ok(())
    }

    fn set_property_value(&mut self, id: LayerId, value: Property) -> Result<(), CoreError> {
        let index = self.index_of(id)?;
        let layer = &mut self.layers[index];
        let mut channel = None;
        let marks = !matches!(value, Property::Name(_) | Property::Locks(_)); // 名前は合成を変えない
        match value {
            Property::Locks(v) => layer.locks = v,
            Property::Visible(v) => layer.visible = v,
            Property::Opacity(v) => layer.opacity = v,
            Property::Mode(v) => layer.blend_mode = v,
            Property::Clipping(v) => layer.clipping = v,
            Property::Name(v) => layer.name = v,
            Property::MaskEnabled(v) => layer.mask.as_mut().expect("マスク").enabled = v,
            Property::MaskInverted(v) => layer.mask.as_mut().expect("マスク").inverted = v,
            Property::MaskDensity(v) => layer.mask.as_mut().expect("マスク").density = v,
            Property::Adjustment(v) => layer.adjustment = Some(v),
            Property::ChannelBlend(c, b) => {
                layer.set_channel_blend(c, b);
                channel = Some(c);
            }
        }
        if marks {
            self.mark_layer(index, channel);
            self.mark_clipped_layers();
        }
        Ok(())
    }

    // ───────── 変化の記録の印 ─────────

    /// 層が変わったことにする（C# の MarkLayerChanged）: ラスターは面のあるタイル、塗りつぶし・調整は画布全体、グループは中身と
    /// グループのマスクのタイル。channel が None なら全チャンネル。グループは自分と子孫の全部を `mark_layer_alone` で 1 回ずつ
    /// （入れ子のグループごとに子孫を数え直すと、入れ子の段数の指数の時間になる）。
    fn mark_layer(&mut self, index: usize, channel: Option<Channel>) {
        let mut targets: Vec<usize> = Vec::new();
        if self.layers[index].is_group() {
            let id = self.layers[index].id;
            targets.extend((0..self.layers.len()).filter(|&i| self.is_descendant(i, id)));
        }
        targets.push(index);
        for i in targets {
            self.mark_layer_alone(i, channel);
        }
    }

    /// 層 1 枚だけの印（グループは自分のマスクのタイルだけ。中身の層は `mark_layer` が子孫として別に呼ぶ）。層ごとにすることは、
    /// 入れ子の中の層にも漏れなく働くよう、ここに置く。
    fn mark_layer_alone(&mut self, index: usize, channel: Option<Channel>) {
        // マスクの Anchor を読む段のために、この層のマスクの出力が変わったことを数える（入れ子の中の層も 1 回ずつ）
        self.note_mask_output(index);
        if self.layers[index].is_group() {
            self.mark_mask(index, channel);
        } else {
            self.mark_layer_object(index, None, channel);
        }
    }

    /// 層のマスクのタイルを変わったことにする（マスクが無ければ何もしない）。
    fn mark_mask(&mut self, index: usize, channel: Option<Channel>) {
        if let Some(m) = &self.layers[index].mask {
            let coords = m.surface.tile_coords();
            for c in self.mark_targets(channel) {
                for coord in &coords {
                    self.journal.mark(c, *coord);
                }
            }
        }
    }

    /// グループでない層の印（層は文書の index か、文書の外の object）。
    fn mark_layer_object(
        &mut self,
        index: usize,
        object: Option<&Layer>,
        channel: Option<Channel>,
    ) {
        // 文書の外の層を渡すとき、文書の層が 1 枚も無くても番号は引かない（全部の層を外す交換のとき）
        let layer = match object {
            Some(layer) => layer,
            None => &self.layers[index],
        };
        if layer.kind != LayerKind::Raster {
            // 塗りつぶし・調整は画布全体に効く。値が今消えたかもしれないので、今の中身ではなく全タイルを
            let tiles: Vec<TileCoord> = self.canvas_tiles().collect();
            for c in self.mark_targets(channel) {
                for coord in &tiles {
                    self.journal.mark(c, *coord);
                }
            }
            return;
        }
        let (cols, rows) = (
            self.width.div_ceil(self.tile_size),
            self.height.div_ceil(self.tile_size),
        );
        let mut marks = Vec::new();
        for c in layer.surface_channels() {
            if channel.is_none() || channel == Some(c) {
                // フィルターのある層は、ぼかしが透明へ広げる分のタイルにも出力がある
                let expansion: u32 = layer
                    .active_chain(c)
                    .iter()
                    .filter(|e| e.settings.expands_coverage())
                    .map(|e| e.settings.halo())
                    .sum();
                let m = expansion.div_ceil(self.tile_size);
                for coord in layer.surface(c).expect("面").tile_coords() {
                    if m == 0 {
                        marks.push((c, coord));
                        continue;
                    }
                    for y in coord.y.saturating_sub(m)..=(coord.y + m).min(rows - 1) {
                        for x in coord.x.saturating_sub(m)..=(coord.x + m).min(cols - 1) {
                            marks.push((c, TileCoord::new(x, y)));
                        }
                    }
                }
            }
        }
        for (c, coord) in marks {
            self.journal.mark(c, coord);
        }
    }

    /// 層（文書の中か、段の持つ層）を変わったことにする。
    fn mark_layer_anywhere(&mut self, id: LayerId, spare: &[Layer], channel: Option<Channel>) {
        if let Some(i) = self.layer_index(id) {
            self.mark_layer(i, channel);
        } else if let Some(l) = spare.iter().find(|l| l.id == id) {
            if l.is_group() {
                // 文書の外のグループ: 中身はもう文書の中で（別に印を付ける）、マスクのタイルだけ
                if let Some(m) = &l.mask {
                    let coords = m.surface.tile_coords();
                    for c in self.mark_targets(channel) {
                        for coord in &coords {
                            self.journal.mark(c, *coord);
                        }
                    }
                }
            } else {
                self.mark_layer_object(0, Some(l), channel);
            }
        }
    }

    fn mark_targets(&self, channel: Option<Channel>) -> Vec<Channel> {
        match channel {
            Some(c) => vec![c],
            None => self.channels(),
        }
    }

    /// クリッピングの印のある層（兄弟の一番下も）を全部変わったことにする: 並べ替え・表示などで下地が変わると、見える所が変わる。
    /// 下地がグループなら、その中身も（クリッピングの組の有無で通過と分離が入れ替わる）。
    fn mark_clipped_layers(&mut self) {
        for i in 0..self.layers.len() {
            if self.layers[i].clipping {
                self.mark_layer(i, None);
            }
        }
        self.mark_clip_bases();
    }

    /// クリッピングの印のある層の下地（同じグループのすぐ下の、印の無い兄弟）のうち、グループのものの中身に印を付ける。
    /// グループはクリッピングの組を持つと通過でも分離で合成するので、組の層が出入りする（印・表示・チャンネル・面・並び）と
    /// 中身の合成が変わり得る（C# はここを記録しない）。
    fn mark_clip_bases(&mut self) {
        let mut bases = Vec::new();
        for i in 0..self.layers.len() {
            if !self.layers[i].clipping {
                continue;
            }
            let parent = self.layers[i].parent;
            let base = (0..i)
                .rev()
                .filter(|&j| self.layers[j].parent == parent)
                .find(|&j| !self.layers[j].clipping);
            if let Some(b) = base {
                if self.layers[b].is_group() && !bases.contains(&b) {
                    bases.push(b);
                }
            }
        }
        for b in bases {
            self.mark_layer(b, None);
        }
    }

    /// マスクのタイルは、層が覆うどのチャンネルの合成も変え得る（C# の MarkMaskTileChanged と CoveredChannels）。
    fn mark_mask_tile(&mut self, index: usize, coord: TileCoord) {
        let channels = self.covered_channels(index);
        for &c in &channels {
            self.journal.mark(c, coord);
        }
        // マスクのフィルターがあれば、隠す量の変化はその広がりの分だけ出力へ届く
        if let Some(m) = &self.layers[index].mask {
            let chain: Vec<&FilterEffect> = m.filters.iter().filter(|e| e.is_active()).collect();
            if !chain.is_empty() {
                let global = chain.iter().any(|e| e.settings.is_global());
                let halo: u32 = chain.iter().map(|e| e.settings.halo()).sum();
                self.mark_reach(&channels, coord, halo, global);
            }
        }
    }

    /// 層が合成を変え得るチャンネル: 塗りつぶしは値のあるもの、調整は有効なもの、グループは全部、ラスターは面のあるもの。
    fn covered_channels(&self, index: usize) -> Vec<Channel> {
        let l = &self.layers[index];
        match l.kind {
            LayerKind::Fill => l.fill.keys().copied().collect(),
            LayerKind::Adjustment => l.enabled_channels(),
            LayerKind::Group => self.channels(),
            LayerKind::Raster => l.surface_channels(),
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
        target: Target,
        changes: &[TileChange],
        backwards: bool,
    ) -> Result<(), CoreError> {
        let index = self.index_of(layer)?;
        let growth = self.growth_for(index, target);
        let surface = self
            .target_surface_mut(index, target)
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
            self.mark_target_tile(index, target, c.coord);
        }
        Ok(())
    }

    /// 層の元画素（チャンネルの面かマスク）のタイルが変わった。元画素の変化の記録（評価のキャッシュの鍵）と、合成が変わり得るタイル
    /// （フィルターがあれば、その広がりの分も）を記録する。
    fn mark_target_tile(&mut self, index: usize, target: Target, coord: TileCoord) {
        self.note_source(index, target, coord);
        match target {
            Target::Channel(c) => {
                self.journal.mark(c, coord);
                let chain = self.layers[index].active_chain(c);
                if !chain.is_empty() {
                    let global = chain.iter().any(|e| e.settings.is_global());
                    let halo: u32 = chain.iter().map(|e| e.settings.halo()).sum();
                    self.mark_reach(&[c], coord, halo, global);
                }
            }
            Target::Mask => self.mark_mask_tile(index, coord),
        }
    }

    /// タイルの変化が出力へ届く範囲（半径 halo の分のタイル。全域の段があれば画布全体）を、チャンネルに記録する。
    fn mark_reach(&mut self, channels: &[Channel], coord: TileCoord, halo: u32, global: bool) {
        let (cols, rows) = (
            self.width.div_ceil(self.tile_size),
            self.height.div_ceil(self.tile_size),
        );
        if global {
            for &c in channels {
                for y in 0..rows {
                    for x in 0..cols {
                        self.journal.mark(c, TileCoord::new(x, y));
                    }
                }
            }
            return;
        }
        let m = halo.div_ceil(self.tile_size);
        if m == 0 {
            return;
        }
        for &c in channels {
            for y in coord.y.saturating_sub(m)..=(coord.y + m).min(rows - 1) {
                for x in coord.x.saturating_sub(m)..=(coord.x + m).min(cols - 1) {
                    self.journal.mark(c, TileCoord::new(x, y));
                }
            }
        }
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

    /// チャンネルを選んでストロークを始める（ラスターの層だけ。面が無ければ作って有効にする。無効のチャンネルは断る）。
    pub fn begin_stroke_in(
        &mut self,
        layer: LayerId,
        channel: Channel,
        brush: &BrushSettings,
    ) -> Result<Stroke, CoreError> {
        self.begin_brush_stroke_in(layer, channel, &Brush::from(*brush))
    }

    /// 全部入りのブラシ（筆先・ゆらぎ・質感・デュアル・色の変化・フェードと傾き・手ぶれ補正・入り抜き・曲線・効果）で
    /// Color のチャンネルへのストロークを始める。設定はここで写して固定する。
    pub fn begin_brush_stroke(
        &mut self,
        layer: LayerId,
        brush: &Brush,
    ) -> Result<Stroke, CoreError> {
        self.begin_brush_stroke_in(layer, Channel::Color, brush)
    }

    /// 全部入りのブラシで、チャンネルを選んでストロークを始める。色の変化は Color と Emission にだけ効く（ほかのチャンネルは
    /// 値のデータなので、色相などで揺らすと壊すだけ。C# の ForChannel と同じ）。
    pub fn begin_brush_stroke_in(
        &mut self,
        layer: LayerId,
        channel: Channel,
        brush: &Brush,
    ) -> Result<Stroke, CoreError> {
        self.ensure_loadable()?;
        brush.validate()?;
        self.require_channel(channel)?;
        let index = self.index_of(layer)?;
        // 型とロックの順は C# の BeginStroke と同じ: 塗りつぶしの層だけ型が先で、調整・グループの層はロックが先
        // （面を取る GetChannel が型で断るのは、ロックの検査のあと）。そのほかの入口は、ラスターの層かどうかが先
        if self.layers[index].kind == LayerKind::Fill {
            self.ensure_raster(index)?;
        }
        let keep_alpha = self.pixel_write_guard(layer, brush.base.erase)?;
        self.ensure_raster(index)?;
        self.refuse_path_layer(index)?;
        self.ensure_surface(index, channel);
        if !self.layers[index].is_channel_enabled(channel) {
            return Err(CoreError::Unsupported("無効のチャンネルには描けない"));
        }
        let target = Target::Channel(channel);
        let budgets = Budgets {
            growth: self.growth_for(index, target),
            stroke: self.stroke_budget,
        };
        let id = self.next_stroke;
        self.next_stroke += 1;
        let kind = self.channel_kind(channel)?;
        let brush = if crate::brush::carries_color(kind) {
            brush.clone()
        } else {
            brush.without_color_dynamics()
        };
        let size = (self.width, self.height, self.tile_size);
        self.active = Some(
            StrokeState::new(
                id, layer, index, channel, kind, brush, budgets, size, keep_alpha,
            )
            .with_selection(self.selection.clone()),
        );
        self.active_target = target;
        Ok(Stroke { id })
    }

    /// 層のマスクへのストロークを始める: 塗ると隠し、消しゴムで見せる。ブラシの色は使わない（マスクは隠す量だけを持つ）。
    /// 不透明度・流量・硬さ・筆圧はふつうどおり。色の変化とステンシルの色は効かない（C# の ForChannel(null)）。どの種類の層
    /// （グループも）のマスクにも描ける。
    pub fn begin_mask_stroke(
        &mut self,
        layer: LayerId,
        brush: &BrushSettings,
    ) -> Result<Stroke, CoreError> {
        self.begin_brush_mask_stroke(layer, &Brush::from(*brush))
    }

    /// 全部入りのブラシでマスクへのストロークを始める（[`Document::begin_mask_stroke`]）。
    pub fn begin_brush_mask_stroke(
        &mut self,
        layer: LayerId,
        brush: &Brush,
    ) -> Result<Stroke, CoreError> {
        self.ensure_loadable()?;
        brush.validate()?;
        let index = self.index_of(layer)?;
        // マスクの有無がロックより先（C# の RequireMask のあとに RefuseLockedAttributes）。マスクの無い層は、ロックの有無に関わらず同じ理由で断る
        if self.layers[index].mask.is_none() {
            return Err(CoreError::Unsupported("層にマスクが無い"));
        }
        self.refuse_lock(layer, LayerLocks::ALL)?;
        let mut brush = brush.without_color_dynamics();
        brush.base.color = Rgba8::new(0, 0, 0, 255);
        let budgets = Budgets {
            growth: self.growth_for(index, Target::Mask),
            stroke: self.stroke_budget,
        };
        let id = self.next_stroke;
        self.next_stroke += 1;
        let size = (self.width, self.height, self.tile_size);
        // StrokeState のチャンネルはマスクでは使わない（描く面は active_target が決める）
        self.active = Some(
            StrokeState::new(
                id,
                layer,
                index,
                Channel::Color,
                ChannelKind::Scalar,
                brush,
                budgets,
                size,
                false, // マスクは透明部分のロックの対象外（すべてのロックだけで断る。上の refuse_lock）
            )
            .without_stencil_colour()
            .with_selection(self.selection.clone()),
        );
        self.active_target = Target::Mask;
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
            state.apply_pixel(surface, x, y, coverage, pressure, None, changed)
        })
    }

    /// 進行中のストロークと面へ f を当て、変わったタイルを記録する。失敗したらストロークを取り消してから返す。
    fn with_stroke<F>(&mut self, id: u64, mut f: F) -> Result<bool, CoreError>
    where
        F: FnMut(&mut StrokeState, &mut Surface, &mut Vec<TileCoord>) -> Result<bool, CoreError>,
    {
        if !self.material.extra.is_empty() {
            return self.with_material_stroke(id, f);
        }
        let target = self.active_target;
        let state = match self.active.as_mut() {
            Some(a) if a.id == id => a,
            _ => return Err(CoreError::NoActiveStroke),
        };
        let index = state.layer_index;
        debug_assert!(target == Target::Mask || target == Target::Channel(state.channel));
        let surface = match target {
            Target::Channel(c) => self.layers[index].surface_mut(c),
            Target::Mask => self.layers[index].mask.as_mut().map(|m| &mut m.surface),
        }
        .expect("ストロークの面");
        let mut changed = Vec::new();
        let result = f(state, surface, &mut changed);
        for coord in changed {
            self.mark_target_tile(index, target, coord);
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

    /// ストロークを確定する: 待たせていた入力（手ぶれ補正の糸の先・曲線の最後の区間・抜きのダブ）を描き、変わったタイルの前後を
    /// 1 回の Undo として積む。何も変わらなければ積まない（Redo も残す）。待たせていた入力が予算などで断られたら、取り消して返す。
    pub fn end_stroke(&mut self, stroke: Stroke) -> Result<StrokeResult, CoreError> {
        match &self.active {
            Some(a) if a.id == stroke.id => {}
            _ => return Err(CoreError::NoActiveStroke),
        }
        if self.triangle_fill.is_some() {
            return Ok(self.finish_triangles(false));
        }
        self.with_stroke(stroke.id, |state, surface, changed| {
            state.finish_input(surface, changed)
        })?;
        if self.material.started {
            return self.finish_material();
        }
        let state = self.active.take().expect("確かめた");
        let target = self.active_target;
        let surface = self
            .target_surface_mut(state.layer_index, target)
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
                kind: HistoryKind::Brush,
                command: Command::Stroke {
                    layer: state.layer,
                    target,
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
        if self.triangle_fill.is_some() {
            self.finish_triangles(true);
            return true;
        }
        if self.material.started {
            return self.cancel_material();
        }
        let Some(state) = self.active.take() else {
            return false;
        };
        let target = self.active_target;
        let surface = self
            .target_surface_mut(state.layer_index, target)
            .expect("ストロークの面");
        let mut restored = false;
        let mut coords: Vec<TileCoord> = state.tiles.keys().copied().collect();
        coords.sort();
        for coord in &coords {
            surface.restore(*coord, state.tiles[coord].before.as_ref());
            restored = true;
        }
        for coord in coords {
            self.mark_target_tile(state.layer_index, target, coord);
        }
        if restored {
            self.revision += 1;
        }
        true
    }

    // ───────── 合成 ─────────

    /// Color の合成（straight RGBA8、行は下から上）。
    pub fn composite(&self, rect: Rect) -> Result<Vec<u8>, CoreError> {
        self.composite_channel(Channel::Color, rect)
    }

    /// チャンネルの合成（straight RGBA8、行は下から上）。
    pub fn composite_channel(&self, channel: Channel, rect: Rect) -> Result<Vec<u8>, CoreError> {
        let mut out = vec![0u8; rect.width as usize * rect.height as usize * 4];
        self.composite_into(channel, rect, &mut out, RowOrder::BottomUp)?;
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
        self.composite_into_cancellable(channel, rect, out, order, None)
    }

    /// `composite_into` の、効果の評価（フィルター・Generator・画像・Anchor）を取り消せる形。cancel が立つと `Cancelled` で戻り、
    /// 出力は書き換えない。評価し終えたブロックはキャッシュに残る（途中の画素は持たない）。
    pub fn composite_into_cancellable(
        &self,
        channel: Channel,
        rect: Rect,
        out: &mut [u8],
        order: RowOrder,
        cancel: Option<&std::sync::atomic::AtomicBool>,
    ) -> Result<(), CoreError> {
        self.check_rect(rect)?;
        let kind = self.channel_kind(channel)?;
        if out.len() != rect.width as usize * rect.height as usize * 4 {
            return Err(CoreError::InvalidArgument("出力の大きさが矩形と違う"));
        }
        let eval = if rect.is_empty() {
            eval::EvalSet::default()
        } else {
            self.evaluate_for_composite(channel, kind, rect, cancel)?
        };
        let stack = Stack::new(&self.layers, channel, kind, Some(&eval));
        composite::composite_into(&stack, self.tile_size, rect, out, order);
        Ok(())
    }

    /// 層 `id` より下の合成（その層と上の層は入れない。調整の層の入力の見積りに使う）。straight RGBA8、行は下から上。
    /// 層の並びの前の部分だけで合成するので、入れ子の層は、親のグループがその層より前にある並びの範囲で合成する。
    pub fn composite_below(
        &self,
        id: LayerId,
        channel: Channel,
        rect: Rect,
    ) -> Result<Vec<u8>, CoreError> {
        self.check_rect(rect)?;
        let index = self.index_of(id)?;
        let kind = self.channel_kind(channel)?;
        let mut out = vec![0u8; rect.width as usize * rect.height as usize * 4];
        if rect.is_empty() {
            return Ok(out);
        }
        let eval = self.evaluate_for_composite(channel, kind, rect, None)?;
        let stack = Stack::new(&self.layers[..index], channel, kind, Some(&eval));
        composite::composite_into(&stack, self.tile_size, rect, &mut out, RowOrder::BottomUp);
        Ok(out)
    }

    /// 1 画素の合成（参照の式。画素ごとに層を引くので遅い。試験・スポイト向け）。
    pub fn composite_pixel(&self, channel: Channel, x: u32, y: u32) -> Result<Rgba8, CoreError> {
        if x >= self.width || y >= self.height {
            return Err(CoreError::InvalidArgument("画素が画布の外"));
        }
        let kind = self.channel_kind(channel)?;
        let ts = self.tile_size;
        let eval = self.evaluate_for_composite(
            channel,
            kind,
            Rect::new(x / ts * ts, y / ts * ts, 1, 1),
            None,
        )?;
        let stack = Stack::new(&self.layers, channel, kind, Some(&eval));
        Ok(composite::composite_pixel(&stack, x, y))
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
    /// 読み込み・マスク）と、層の並び・入れ子・表示・不透明度・モード・クリッピング・チャンネル・追加・削除のときはその層の持つ
    /// タイル全部（塗りつぶし・調整は画布全体、グループは中身）。元に戻った所を含むことがある。since がこの文書の番号でなければ
    /// None（全部を描き直す）。
    pub fn changed_tiles(&self, channel: Channel, since: u64) -> Option<Vec<TileCoord>> {
        if since > self.journal.serial {
            return None;
        }
        let raw = |c: Channel| -> Vec<TileCoord> {
            self.journal
                .tiles
                .get(c.index())
                .map(|m| {
                    m.iter()
                        .filter(|(_, &s)| s > since)
                        .map(|(c, _)| *c)
                        .collect()
                })
                .unwrap_or_default()
        };
        let mut v = if self.has_anchor_readers() {
            // Anchor を読む段の出力は、読む Anchor が変わった所（段より後の半径の分だけ広げて）も変わる
            let mut sets: HashMap<Channel, std::collections::HashSet<TileCoord>> = self
                .channels()
                .into_iter()
                .map(|c| (c, raw(c).into_iter().collect()))
                .collect();
            self.close_over_anchor_readers(since, &mut sets);
            sets.remove(&channel)
                .map(|s| s.into_iter().collect())
                .unwrap_or_default()
        } else {
            raw(channel)
        };
        v.sort();
        Some(v)
    }
}
