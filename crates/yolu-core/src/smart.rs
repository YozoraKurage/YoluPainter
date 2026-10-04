//! 名前を付けた層の断片。捕捉した時点の写しを保持し、配置しても変わらない。
use crate::{Channel, ChannelInfo, CoreError, Layer, LayerId};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SmartKind {
    Material,
    Mask,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SmartResampling {
    Nearest,
    Bilinear,
    Area,
}
#[derive(Clone, Debug)]
pub struct SmartMaterial {
    pub(crate) document_id: u128,
    pub(crate) normal_settings: crate::NormalSettings,
    pub(crate) kind: SmartKind,
    pub(crate) name: String,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) tile_size: u32,
    pub(crate) layers: Vec<Layer>,
    pub(crate) channel_info: Vec<Option<ChannelInfo>>,
    /// 捕まえるときに変わったことの知らせ（モデルの上のパスを画素だけにした、など。保存はしない）。
    pub(crate) notes: Vec<String>,
}
impl SmartMaterial {
    pub fn kind(&self) -> SmartKind {
        self.kind
    }
    pub fn name(&self) -> &str {
        &self.name
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
    pub fn layers(&self) -> &[Layer] {
        &self.layers
    }
    pub fn pixel_bytes(&self) -> u64 {
        self.layers.iter().map(Layer::allocated_bytes).sum()
    }
    /// 捕まえるときに変わったことの知らせ。
    pub fn notes(&self) -> &[String] {
        &self.notes
    }
    pub fn channels(&self) -> Vec<Channel> {
        if self.kind == SmartKind::Mask {
            return Vec::new();
        }
        self.channel_info
            .iter()
            .enumerate()
            .filter(|(_, c)| c.is_some())
            .map(|(i, _)| Channel::from_index(i).expect("チャンネル番号"))
            .filter(|&c| self.layers.iter().any(|l| l.is_channel_enabled(c)))
            .collect()
    }
    pub fn renamed(&self, name: &str) -> Result<Self, CoreError> {
        check_name(name)?;
        let mut copy = self.clone();
        copy.name = name.into();
        Ok(copy)
    }
}
pub(crate) fn check_name(name: &str) -> Result<(), CoreError> {
    if name.trim().is_empty()
        || name.encode_utf16().count() > 256
        || name.chars().any(|c| c < ' ' || c == '\x7f')
    {
        return Err(CoreError::InvalidArgument("スマート素材の名前"));
    }
    Ok(())
}
#[derive(Clone, Debug, Default)]
pub struct SmartPlacement {
    pub parent: Option<LayerId>,
    /// None または兄弟数を超える位置は最上位。
    pub position: Option<usize>,
    /// None は元の有効状態を保つ。値と画素は無効化しても保持する。
    pub channels: Option<Vec<Channel>>,
    pub group_name: Option<String>,
    pub resampling: Option<SmartResampling>,
}
#[derive(Clone, Debug)]
pub struct SmartPlaceResult {
    pub layer_id: LayerId,
    pub layers: Vec<LayerId>,
    pub switched_off: Vec<Channel>,
    pub resampled: bool,
    pub replaced_mask: bool,
    /// 置くときに変わったことの知らせ（ぼかし・シャープの半径を倍率に合わせた・丸めた、キャンバスのパスを画素だけにした）。
    pub notes: Vec<String>,
}
