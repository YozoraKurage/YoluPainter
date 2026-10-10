//! レイヤー（C# の PaintLayer・RasterMask・ChannelBlend）。種類はラスター・塗りつぶし・調整・グループ。
//!
//! - 文書のレイヤーは下から上の平らな並びで、グループの中身はグループのすぐ下に続けて並ぶ（PSD のフォルダと同じ）。親は `parent`。
//! - チャンネルごとに: ラスターは面、塗りつぶしは 1 つの値、どの種類も有効の印と、レイヤーの値を置き換える合成モード・不透明度
//!   （[`ChannelBlend`]）。表示・マスク・クリッピングは全チャンネルで共有する。
//! - マスクは隠す量を面のアルファに持つ（RGB は 0）。無いタイルは何も隠さない。

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use crate::adjust::AdjustmentSettings;
use crate::effects::{Anchor, FilterEffect, ImageId, LayerPath};
use crate::error::CoreError;
use crate::fill_image::Projection;
use crate::generator;
use crate::math::UNIT;
use crate::surface::Surface;
use crate::types::{BlendMode, Channel, LayerKind, Rgba8};

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

/// レイヤーのチャンネルごとの合成モード・不透明度（Substance Painter のチャンネルごとの合成）。None の部分はレイヤーの値に従い、
/// 値のある部分はそのチャンネルでだけレイヤーの値を置き換える（掛けない）。
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct ChannelBlend {
    pub mode: Option<BlendMode>,
    pub opacity: Option<f64>,
}

impl ChannelBlend {
    pub const fn new(mode: Option<BlendMode>, opacity: Option<f64>) -> Self {
        ChannelBlend { mode, opacity }
    }
    /// どちらもレイヤーに従う。
    pub fn is_empty(&self) -> bool {
        self.mode.is_none() && self.opacity.is_none()
    }
}

/// レイヤーのラスターマスク（全チャンネルで共有）。隠す量を面のアルファに持つ。塗ると隠し、消すと見せる。
#[derive(Clone, Debug)]
pub struct RasterMask {
    pub(crate) surface: Surface,
    pub(crate) enabled: bool,
    pub(crate) inverted: bool,
    pub(crate) density: f64,
    /// マスクのフィルターのスタック（隠す量にかける。下から上の順に当たる。全チャンネルで共有し、レイヤーの画素のスタックとは別）。
    pub(crate) filters: Vec<FilterEffect>,
    /// このマスクにある Anchor（マスクを外すと一緒に無くなる）。
    pub(crate) anchor: Option<Anchor>,
}

impl RasterMask {
    pub(crate) fn new(surface: Surface) -> Self {
        RasterMask {
            surface,
            enabled: true,
            inverted: false,
            density: 1.0,
            filters: Vec::new(),
            anchor: None,
        }
    }
    /// マスクのフィルターのスタック（下から上。最初が先に当たる）。
    pub fn filters(&self) -> &[FilterEffect] {
        &self.filters
    }
    /// マスクの Anchor（マスクがレイヤーを見せる量。上のレイヤーの Generator が読む）。
    pub fn anchor(&self) -> Option<&Anchor> {
        self.anchor.as_ref()
    }
    /// 結果を変えるフィルターの段（有効で強さが 0 より大きい）があるか。
    pub fn has_active_filters(&self) -> bool {
        self.filters.iter().any(FilterEffect::is_active)
    }
    /// 隠す量の面（アルファ = 隠す量 0〜255、RGB は 0）。
    pub fn surface(&self) -> &Surface {
        &self.surface
    }
    pub fn enabled(&self) -> bool {
        self.enabled
    }
    pub fn inverted(&self) -> bool {
        self.inverted
    }
    /// 濃度 0〜1（0 なら何も隠さない）。
    pub fn density(&self) -> f64 {
        self.density
    }
    /// 隠す量（0〜255）に対して、レイヤーのアルファに掛ける値（C# の Factor と同じ式）。
    #[inline]
    pub fn factor(&self, hide: u8) -> f64 {
        if !self.enabled {
            return 1.0;
        }
        let h = UNIT[hide as usize];
        if self.inverted {
            1.0 - self.density * (1.0 - h)
        } else {
            1.0 - self.density * h
        }
    }
    /// 画素での値。
    pub fn factor_at(&self, x: u32, y: u32) -> Result<f64, CoreError> {
        Ok(self.factor(self.surface.pixel(x, y)?.a))
    }
    /// どの画素も変えないか: 無効・濃度 0・または何も隠さず反転もしない（このときの値はちょうど 1）。有効なフィルターの段があれば、
    /// 空のマスクからも値ができる（反転・ノイズなど）ので、変えないとは言えない。
    pub fn is_neutral(&self) -> bool {
        !self.enabled
            || self.density == 0.0
            || (!self.inverted && self.surface.tile_count() == 0 && !self.has_active_filters())
    }
    /// 隠す量の 256 の表（`factor` そのものの値）。
    pub(crate) fn factor_table(&self) -> [f64; 256] {
        let mut t = [0.0; 256];
        for (h, v) in t.iter_mut().enumerate() {
            *v = self.factor(h as u8);
        }
        t
    }
}

/// レイヤー。種類ごとに中身が違う（ラスターは面、塗りつぶしは値、調整は設定、グループは何も持たない）。
#[derive(Clone, Debug)]
pub struct Layer {
    pub(crate) locks: crate::LayerLocks,
    pub(crate) id: LayerId,
    pub(crate) name: String,
    pub(crate) visible: bool,
    pub(crate) opacity: f64,
    pub(crate) blend_mode: BlendMode,
    pub(crate) clipping: bool,
    pub(crate) kind: LayerKind,
    /// 入っているグループ（None は一番上の段）。
    pub(crate) parent: Option<LayerId>,
    /// チャンネルの番号ごとの面（ラスターだけ。無いチャンネルは None）。
    pub(crate) surfaces: Vec<Option<Surface>>,
    /// 有効なチャンネルの印（bit = チャンネルの番号）。無効にしたチャンネルの画素・値は保つ。
    pub(crate) enabled: u64,
    /// 塗りつぶしのチャンネルごとの値（塗りつぶしだけ）。
    pub(crate) fill: BTreeMap<Channel, Rgba8>,
    /// 調整の設定（調整だけ）。
    pub(crate) adjustment: Option<AdjustmentSettings>,
    pub(crate) mask: Option<RasterMask>,
    /// チャンネルごとの合成（空の設定は持たない）。
    pub(crate) blends: BTreeMap<Channel, ChannelBlend>,
    /// レイヤーの画素のフィルターのスタック（下から上。各段が適用するチャンネルを持つ。ラスターと塗りつぶしだけ）。
    pub(crate) filters: Vec<FilterEffect>,
    /// このレイヤーにある Anchor（そのレイヤーまでのスタックの結果）。
    pub(crate) anchor: Option<Anchor>,
    /// 塗りつぶしのチャンネルごとの画像（プロジェクトの画像リソースの ID。そのチャンネルの値は画像が使えない所に出る）。
    pub(crate) fill_images: BTreeMap<Channel, ImageId>,
    /// 塗りつぶしの画像を異方性のフィルターなしで読むチャンネル（既定は異方性で読む。画像のあるチャンネルだけ）。
    pub(crate) fill_isotropic: BTreeSet<Channel>,
    /// 塗りつぶしの画像の投影（レイヤーで 1 つ。画像が無くてもデカールは投影を使う）。
    pub(crate) projection: Projection,
    /// 塗りつぶしのチャンネルごとのグラデーション（ランプ付きの形のグラデーションの Generator。置き換え）。
    pub(crate) fill_gradients: BTreeMap<Channel, generator::Settings>,
    /// 塗りつぶしのチャンネルごとの点のグラデーション（画像・形のグラデーションとは同じチャンネルに置かない）。
    pub(crate) fill_points: BTreeMap<Channel, crate::fill_points::PointGradient>,
    /// レイヤーの画素を描くパスの一覧（ラスターだけ。対象のチャンネルの画素は、一覧の見せるパスを順に描いた結果）。
    pub(crate) paths: Vec<crate::paths::LayerPathEntry>,
    /// レイヤーの Color の画素を描くテキストの値（ラスターだけ。パスとは両方持たない。画素は値とフォントから描いた結果）。
    pub(crate) text: Option<crate::text::TextSettings>,
    /// このレイヤー（グループ）に付く定規（作った順。どの種類のレイヤーにも付く）。
    pub(crate) rulers: Vec<crate::rulers::Ruler>,
}

impl Layer {
    pub(crate) fn new(id: LayerId, name: &str, kind: LayerKind) -> Layer {
        Layer {
            locks: crate::LayerLocks::NONE,
            id,
            name: name.to_string(),
            visible: true,
            opacity: 1.0,
            blend_mode: if kind == LayerKind::Group {
                BlendMode::PassThrough
            } else {
                BlendMode::Normal
            },
            clipping: false,
            kind,
            parent: None,
            surfaces: Vec::new(),
            enabled: 0,
            fill: BTreeMap::new(),
            adjustment: None,
            mask: None,
            blends: BTreeMap::new(),
            filters: Vec::new(),
            anchor: None,
            fill_images: BTreeMap::new(),
            fill_isotropic: BTreeSet::new(),
            projection: Projection::default(),
            fill_gradients: BTreeMap::new(),
            fill_points: BTreeMap::new(),
            paths: Vec::new(),
            text: None,
            rulers: Vec::new(),
        }
    }

    pub fn locks(&self) -> crate::LayerLocks {
        self.locks
    }
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
    /// すぐ下の兄弟（クリッピングの下地）の中にだけ描く印。兄弟の一番下では効かない（印は保つ）。
    pub fn clipping(&self) -> bool {
        self.clipping
    }
    pub fn kind(&self) -> LayerKind {
        self.kind
    }
    pub fn is_group(&self) -> bool {
        self.kind == LayerKind::Group
    }
    /// 入っているグループ（None は一番上の段）。
    pub fn parent(&self) -> Option<LayerId> {
        self.parent
    }
    /// チャンネルの面（ラスターでなければ、または無ければ None）。
    pub fn surface(&self, channel: Channel) -> Option<&Surface> {
        self.surfaces.get(channel.index()).and_then(|s| s.as_ref())
    }
    pub(crate) fn surface_mut(&mut self, channel: Channel) -> Option<&mut Surface> {
        self.surfaces
            .get_mut(channel.index())
            .and_then(|s| s.as_mut())
    }
    pub(crate) fn put_surface(&mut self, channel: Channel, surface: Option<Surface>) {
        let i = channel.index();
        if self.surfaces.len() <= i {
            if surface.is_none() {
                return;
            }
            self.surfaces.resize_with(i + 1, || None);
        }
        self.surfaces[i] = surface;
        while matches!(self.surfaces.last(), Some(None)) {
            self.surfaces.pop();
        }
    }
    /// 面を持つチャンネル（番号の順）。
    pub fn surface_channels(&self) -> Vec<Channel> {
        self.surfaces
            .iter()
            .enumerate()
            .filter(|(_, s)| s.is_some())
            .map(|(i, _)| Channel::from_index(i).expect("番号は 64 未満"))
            .collect()
    }
    pub fn is_channel_enabled(&self, channel: Channel) -> bool {
        self.enabled & channel.bit() != 0
    }
    /// 有効なチャンネル（番号の順）。
    pub fn enabled_channels(&self) -> Vec<Channel> {
        (0..Channel::MAX)
            .filter(|i| self.enabled & (1u64 << i) != 0)
            .map(|i| Channel::from_index(i).expect("64 未満"))
            .collect()
    }
    pub(crate) fn set_enabled(&mut self, channel: Channel, value: bool) {
        if value {
            self.enabled |= channel.bit();
        } else {
            self.enabled &= !channel.bit();
        }
    }
    /// 塗りつぶしのチャンネルの値（塗りつぶしでなければ、または無ければ None）。
    pub fn fill_value(&self, channel: Channel) -> Option<Rgba8> {
        self.fill.get(&channel).copied()
    }
    /// 塗りつぶしの値のあるチャンネルと値（番号の順）。
    pub fn fill_values(&self) -> impl Iterator<Item = (Channel, Rgba8)> + '_ {
        self.fill.iter().map(|(c, v)| (*c, *v))
    }
    /// 調整の設定（調整レイヤーだけ）。
    pub fn adjustment(&self) -> Option<&AdjustmentSettings> {
        self.adjustment.as_ref()
    }
    /// ラスターマスク（無ければ None）。
    pub fn mask(&self) -> Option<&RasterMask> {
        self.mask.as_ref()
    }
    /// そのチャンネルのレイヤー自身の合成の設定（レイヤーに従うなら空）。
    pub fn channel_blend(&self, channel: Channel) -> ChannelBlend {
        self.blends.get(&channel).copied().unwrap_or_default()
    }
    /// チャンネルごとの設定を持つチャンネルと設定。
    pub fn channel_blends(&self) -> impl Iterator<Item = (Channel, ChannelBlend)> + '_ {
        self.blends.iter().map(|(c, b)| (*c, *b))
    }
    /// そのチャンネルで合成に使うモード（チャンネルの設定があればそれ、なければレイヤーの）。
    pub fn blend_mode_in(&self, channel: Channel) -> BlendMode {
        self.blends
            .get(&channel)
            .and_then(|b| b.mode)
            .unwrap_or(self.blend_mode)
    }
    /// そのチャンネルで合成に使う不透明度。
    pub fn opacity_in(&self, channel: Channel) -> f64 {
        self.blends
            .get(&channel)
            .and_then(|b| b.opacity)
            .unwrap_or(self.opacity)
    }
    pub(crate) fn set_channel_blend(&mut self, channel: Channel, blend: ChannelBlend) {
        if blend.is_empty() {
            self.blends.remove(&channel);
        } else {
            self.blends.insert(channel, blend);
        }
    }

    /// レイヤーの画素のフィルターのスタック（下から上。最初が先に当たる）。
    pub fn filters(&self) -> &[FilterEffect] {
        &self.filters
    }
    /// そのチャンネルに当たる有効な段（スタックの順）。
    pub(crate) fn active_chain(&self, channel: Channel) -> Vec<&FilterEffect> {
        self.filters
            .iter()
            .filter(|e| e.is_active() && e.applies_to(channel))
            .collect()
    }
    pub fn has_active_filters(&self, channel: Channel) -> bool {
        self.filters
            .iter()
            .any(|e| e.is_active() && e.applies_to(channel))
    }
    /// レイヤーの画素を描いているパスの一覧の、最初のパス（無ければ None。パスレイヤーかを見るだけなら [`Layer::has_paths`]）。
    pub fn path(&self) -> Option<&LayerPath> {
        self.paths.first().map(|e| &e.path)
    }
    /// レイヤーの画素を描いているパスの一覧（下から順に描く）。
    pub fn paths(&self) -> &[crate::paths::LayerPathEntry] {
        &self.paths
    }
    /// パスで描かれたレイヤーか（一覧が空でない）。
    pub fn has_paths(&self) -> bool {
        !self.paths.is_empty()
    }
    /// このレイヤー（グループ）に付く定規（作った順。同じ描く所の特殊定規の取り合いは、後ろが勝つ）。
    pub fn rulers(&self) -> &[crate::rulers::Ruler] {
        &self.rulers
    }
    /// テキストレイヤーの値（テキストレイヤーでなければ None）。
    pub fn text(&self) -> Option<&crate::text::TextSettings> {
        self.text.as_ref()
    }
    /// このレイヤーの Anchor（そのレイヤーまでのスタックの結果）。
    pub fn anchor(&self) -> Option<&Anchor> {
        self.anchor.as_ref()
    }
    /// 塗りつぶしのチャンネルが読む画像（無ければ None）。
    pub fn fill_image(&self, channel: Channel) -> Option<ImageId> {
        self.fill_images.get(&channel).copied()
    }
    /// 塗りつぶしのチャンネルの画像を異方性のフィルターで読むか（既定は読む）。
    pub fn fill_anisotropic(&self, channel: Channel) -> bool {
        !self.fill_isotropic.contains(&channel)
    }
    /// 画像を持つチャンネルと画像（番号の順）。
    pub fn fill_images(&self) -> impl Iterator<Item = (Channel, ImageId)> + '_ {
        self.fill_images.iter().map(|(c, i)| (*c, *i))
    }
    /// 内容とマスクのフィルターのスタックの、画像の段（Generator の種類 Image）が読む画像（選んでいない段は除く。無効な段も含む）。
    pub fn generator_images(&self) -> impl Iterator<Item = ImageId> + '_ {
        self.filters
            .iter()
            .chain(self.mask.iter().flat_map(|m| m.filters.iter()))
            .filter_map(|e| e.settings.generator_settings())
            .filter(|g| g.kind == generator::Kind::Image && g.image.image != 0)
            .map(|g| ImageId(g.image.image))
    }
    /// このレイヤーが読むプロジェクトの画像（塗りつぶしの画像と画像の段。重なりうる）。
    pub fn image_ids(&self) -> impl Iterator<Item = ImageId> + '_ {
        self.fill_images()
            .map(|(_, id)| id)
            .chain(self.generator_images())
    }
    /// 塗りつぶしの画像の投影。
    pub fn projection(&self) -> &Projection {
        &self.projection
    }
    /// 塗りつぶしのチャンネルのグラデーション。
    pub fn fill_gradient(&self, channel: Channel) -> Option<&generator::Settings> {
        self.fill_gradients.get(&channel)
    }
    /// グラデーションを持つチャンネルと設定（番号の順）。
    pub fn fill_gradients(&self) -> impl Iterator<Item = (Channel, &generator::Settings)> + '_ {
        self.fill_gradients.iter().map(|(c, g)| (*c, g))
    }
    /// 塗りつぶしのチャンネルの点のグラデーション。
    pub fn fill_points(&self, channel: Channel) -> Option<&crate::fill_points::PointGradient> {
        self.fill_points.get(&channel)
    }
    /// 点のグラデーションを持つチャンネルと設定（番号の順）。
    pub fn fill_point_gradients(
        &self,
    ) -> impl Iterator<Item = (Channel, &crate::fill_points::PointGradient)> + '_ {
        self.fill_points.iter().map(|(c, g)| (*c, g))
    }
    /// 投影がデカールの塗りつぶしか。
    pub fn is_decal(&self) -> bool {
        self.kind == LayerKind::Fill
            && self.projection.mode == crate::fill_image::ProjectionMode::Decal
    }
    /// 画素が投影から来る塗りつぶしのチャンネルか: 画像のあるチャンネルと、デカールの値のある全チャンネル（C# の `IsProjectedFill`）。
    pub(crate) fn is_projected_fill(&self, channel: Channel) -> bool {
        self.kind == LayerKind::Fill
            && (self.fill_images.contains_key(&channel)
                || self.is_decal() && self.fill.contains_key(&channel))
    }
    /// レイヤーのそのチャンネルの画素が、保存した値でなく評価で決まるか（有効なフィルター・グラデーション・投影。C# の `HasEvaluatedOutput`）。
    pub fn has_evaluated_output(&self, channel: Channel) -> bool {
        self.has_active_filters(channel)
            || self.kind == LayerKind::Fill && self.fill_gradients.contains_key(&channel)
            || self.kind == LayerKind::Fill && self.fill_points.contains_key(&channel)
            || self.is_projected_fill(channel)
            || self.fill_paths_draw(channel)
    }
    /// 塗りつぶしレイヤーのパスがこのチャンネルを描くか（塗りつぶし → 効果のスタック → パスの順に評価する）。
    pub(crate) fn fill_paths_draw(&self, channel: Channel) -> bool {
        self.kind == LayerKind::Fill
            && !self.paths.is_empty()
            && crate::paths::list_channels(&self.paths).contains(&channel)
    }
    /// 塗りつぶしが焼いたメッシュマップ・画像を読むか（C# の `ReadsMeshMapsForFill` と、画像のレイヤー）。
    pub(crate) fn reads_inputs_for_fill(&self) -> bool {
        self.kind == LayerKind::Fill
            && (!self.fill_gradients.is_empty()
                || !self.fill_images.is_empty()
                || self.is_decal()
                || self
                    .fill_points
                    .values()
                    .any(|g| g.space == crate::fill_points::PointSpace::Model))
    }

    /// そのチャンネルについて何かを持つか: 有効の印・面・塗りつぶしの値・チャンネルごとの合成のどれか。無効にしても面・値・合成は
    /// 残るので、有効かどうかでは決まらない（`has_content` は合成に出せるかで、これは保存や移すときに落としてはいけないかの問い）。
    pub(crate) fn has_channel_data(&self, channel: Channel) -> bool {
        self.is_channel_enabled(channel)
            || self.surface(channel).is_some()
            || self.fill.contains_key(&channel)
            || self.blends.contains_key(&channel)
    }

    /// そのチャンネルに何かを出せるか: 面・塗りつぶしの値・そのチャンネルに使える有効な調整（C# の HasContent）。
    /// グループは中身が決める（ここでは false）。
    pub(crate) fn has_content(&self, channel: Channel, applies: bool) -> bool {
        match self.kind {
            LayerKind::Fill => self.fill.contains_key(&channel) || self.fill_paths_draw(channel),
            LayerKind::Adjustment => {
                self.adjustment.is_some() && applies && self.is_channel_enabled(channel)
            }
            LayerKind::Group => false,
            LayerKind::Raster => self.surface(channel).is_some(),
        }
    }

    /// レイヤーそのものの画素（マスク・不透明度・合成の前）。中身の無い所・調整・グループは透明。
    pub fn pixel(&self, channel: Channel, x: u32, y: u32) -> Result<Rgba8, CoreError> {
        match self.kind {
            LayerKind::Raster => match self.surface(channel) {
                Some(s) => s.pixel(x, y),
                None => Ok(Rgba8::TRANSPARENT),
            },
            LayerKind::Fill => Ok(self.fill_value(channel).unwrap_or(Rgba8::TRANSPARENT)),
            _ => Ok(Rgba8::TRANSPARENT),
        }
    }
    /// 全チャンネルの面とマスクの画素のバイト数。
    pub fn allocated_bytes(&self) -> u64 {
        self.surfaces
            .iter()
            .flatten()
            .map(|s| s.allocated_bytes())
            .sum::<u64>()
            + self
                .mask
                .as_ref()
                .map_or(0, |m| m.surface.allocated_bytes())
    }
}
