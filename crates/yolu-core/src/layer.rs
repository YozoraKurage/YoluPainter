//! 層（C# の PaintLayer・RasterMask・ChannelBlend）。種類はラスター・塗りつぶし・調整・グループ。
//!
//! - 文書の層は下から上の平らな並びで、グループの中身はグループのすぐ下に続けて並ぶ（PSD のフォルダと同じ）。親は `parent`。
//! - チャンネルごとに: ラスターは面、塗りつぶしは 1 つの値、どの種類も有効の印と、層の値を置き換える合成モード・不透明度
//!   （[`ChannelBlend`]）。表示・マスク・クリッピングは全チャンネルで共有する。
//! - マスクは隠す量を面のアルファに持つ（RGB は 0）。無いタイルは何も隠さない。

use std::collections::BTreeMap;
use std::fmt;

use crate::adjust::AdjustmentSettings;
use crate::error::CoreError;
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

/// 層のチャンネルごとの合成モード・不透明度（Substance Painter のチャンネルごとの合成）。None の部分は層の値に従い、
/// 値のある部分はそのチャンネルでだけ層の値を置き換える（掛けない）。
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct ChannelBlend {
    pub mode: Option<BlendMode>,
    pub opacity: Option<f64>,
}

impl ChannelBlend {
    pub const fn new(mode: Option<BlendMode>, opacity: Option<f64>) -> Self {
        ChannelBlend { mode, opacity }
    }
    /// どちらも層に従う。
    pub fn is_empty(&self) -> bool {
        self.mode.is_none() && self.opacity.is_none()
    }
}

/// 層のラスターマスク（全チャンネルで共有）。隠す量を面のアルファに持つ。塗ると隠し、消すと見せる。
#[derive(Clone, Debug)]
pub struct RasterMask {
    pub(crate) surface: Surface,
    pub(crate) enabled: bool,
    pub(crate) inverted: bool,
    pub(crate) density: f64,
}

impl RasterMask {
    pub(crate) fn new(surface: Surface) -> Self {
        RasterMask {
            surface,
            enabled: true,
            inverted: false,
            density: 1.0,
        }
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
    /// 隠す量（0〜255）に対して、層のアルファに掛ける値（C# の Factor と同じ式）。
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
    /// どの画素も変えないか: 無効・濃度 0・または何も隠さず反転もしない（このときの値はちょうど 1）。
    pub fn is_neutral(&self) -> bool {
        !self.enabled || self.density == 0.0 || (!self.inverted && self.surface.tile_count() == 0)
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

/// 層。種類ごとに中身が違う（ラスターは面、塗りつぶしは値、調整は設定、グループは何も持たない）。
#[derive(Clone, Debug)]
pub struct Layer {
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
}

impl Layer {
    pub(crate) fn new(id: LayerId, name: &str, kind: LayerKind) -> Layer {
        Layer {
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
        }
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
    /// 調整の設定（調整の層だけ）。
    pub fn adjustment(&self) -> Option<&AdjustmentSettings> {
        self.adjustment.as_ref()
    }
    /// ラスターマスク（無ければ None）。
    pub fn mask(&self) -> Option<&RasterMask> {
        self.mask.as_ref()
    }
    /// そのチャンネルの層自身の合成の設定（層に従うなら空）。
    pub fn channel_blend(&self, channel: Channel) -> ChannelBlend {
        self.blends.get(&channel).copied().unwrap_or_default()
    }
    /// チャンネルごとの設定を持つチャンネルと設定。
    pub fn channel_blends(&self) -> impl Iterator<Item = (Channel, ChannelBlend)> + '_ {
        self.blends.iter().map(|(c, b)| (*c, *b))
    }
    /// そのチャンネルで合成に使うモード（チャンネルの設定があればそれ、なければ層の）。
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
            LayerKind::Fill => self.fill.contains_key(&channel),
            LayerKind::Adjustment => {
                self.adjustment.is_some() && applies && self.is_channel_enabled(channel)
            }
            LayerKind::Group => false,
            LayerKind::Raster => self.surface(channel).is_some(),
        }
    }

    /// 層そのものの画素（マスク・不透明度・合成の前）。中身の無い所・調整・グループは透明。
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
    pub(crate) fn pixel_or_transparent(&self, channel: Channel, x: u32, y: u32) -> Rgba8 {
        self.pixel(channel, x, y).unwrap_or(Rgba8::TRANSPARENT)
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
