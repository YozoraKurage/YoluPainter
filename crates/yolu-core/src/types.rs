//! 画素・タイルの座標・合成モード・チャンネルなど、文書のどこでも使う小さな型。
//! 値の並び（合成モード・チャンネルの番号）は Unity 版の C# の Core（PaintTypes.cs）と同じで、保存形式に入るので並べ替えない。

use std::cmp::Ordering;
use std::fmt;

/// straight（プリマルチプライドでない）RGBA8。アルファは線形の被覆率、RGB は色の変換をせずにそのまま持つ。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
#[repr(C)]
pub struct Rgba8 {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba8 {
    pub const TRANSPARENT: Rgba8 = Rgba8 {
        r: 0,
        g: 0,
        b: 0,
        a: 0,
    };
    pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        Rgba8 { r, g, b, a }
    }
    /// 4 バイトの並び（R, G, B, A）から。
    #[inline]
    pub fn from_slice(bytes: &[u8]) -> Self {
        Rgba8 {
            r: bytes[0],
            g: bytes[1],
            b: bytes[2],
            a: bytes[3],
        }
    }
    #[inline]
    pub fn to_array(self) -> [u8; 4] {
        [self.r, self.g, self.b, self.a]
    }
}

impl fmt::Debug for Rgba8 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "RGBA({},{},{},{})", self.r, self.g, self.b, self.a)
    }
}

/// タイルの座標（画素の座標 ÷ タイルの大きさ）。左下が (0, 0)。並びは Y、次に X（C# の TileCoord.CompareTo と同じ）。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct TileCoord {
    pub x: u32,
    pub y: u32,
}

impl TileCoord {
    pub const fn new(x: u32, y: u32) -> Self {
        TileCoord { x, y }
    }
}

impl Ord for TileCoord {
    fn cmp(&self, other: &Self) -> Ordering {
        self.y.cmp(&other.y).then(self.x.cmp(&other.x))
    }
}
impl PartialOrd for TileCoord {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// 文書の中の画素の矩形。原点は左下、x・y は左下の画素。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
    pub const fn new(x: u32, y: u32, width: u32, height: u32) -> Self {
        Rect {
            x,
            y,
            width,
            height,
        }
    }
    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }
}

/// 合成の結果の行の並び。文書は左下原点なので `BottomUp`（最初の行が一番下）が C# の Core と同じ並び。
/// 画面（egui のテクスチャなど）へ渡すときは `TopDown`。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum RowOrder {
    #[default]
    BottomUp,
    TopDown,
}

/// レイヤーの合成モード（Photoshop の分類と並び）。値は保存形式に入るので、並べ替えず末尾に足す。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default, PartialOrd, Ord)]
#[repr(u8)]
pub enum BlendMode {
    #[default]
    Normal = 0,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    ColorDodge,
    ColorBurn,
    LinearDodge,
    LinearBurn,
    HardLight,
    SoftLight,
    VividLight,
    LinearLight,
    PinLight,
    HardMix,
    Difference,
    Exclusion,
    Subtract,
    Divide,
    Hue,
    Saturation,
    Color,
    Luminosity,
    DarkerColor,
    LighterColor,
    /// グループだけ: 中身をグループでないかのように下へ重ねる（通過）。グループでないレイヤーには付けられない。
    PassThrough,
}

impl BlendMode {
    /// レイヤーに付けられる 26 のモード（PassThrough を除く）を番号の順に。
    pub const LAYER_MODES: [BlendMode; 26] = [
        BlendMode::Normal,
        BlendMode::Multiply,
        BlendMode::Screen,
        BlendMode::Overlay,
        BlendMode::Darken,
        BlendMode::Lighten,
        BlendMode::ColorDodge,
        BlendMode::ColorBurn,
        BlendMode::LinearDodge,
        BlendMode::LinearBurn,
        BlendMode::HardLight,
        BlendMode::SoftLight,
        BlendMode::VividLight,
        BlendMode::LinearLight,
        BlendMode::PinLight,
        BlendMode::HardMix,
        BlendMode::Difference,
        BlendMode::Exclusion,
        BlendMode::Subtract,
        BlendMode::Divide,
        BlendMode::Hue,
        BlendMode::Saturation,
        BlendMode::Color,
        BlendMode::Luminosity,
        BlendMode::DarkerColor,
        BlendMode::LighterColor,
    ];

    /// 保存形式の番号から（範囲外は None）。
    pub fn from_index(index: u8) -> Option<BlendMode> {
        if (index as usize) < Self::LAYER_MODES.len() {
            Some(Self::LAYER_MODES[index as usize])
        } else if index == BlendMode::PassThrough as u8 {
            Some(BlendMode::PassThrough)
        } else {
            None
        }
    }

    /// C# の列挙の名前（"Multiply" など）。
    pub fn name(self) -> &'static str {
        match self {
            BlendMode::Normal => "Normal",
            BlendMode::Multiply => "Multiply",
            BlendMode::Screen => "Screen",
            BlendMode::Overlay => "Overlay",
            BlendMode::Darken => "Darken",
            BlendMode::Lighten => "Lighten",
            BlendMode::ColorDodge => "ColorDodge",
            BlendMode::ColorBurn => "ColorBurn",
            BlendMode::LinearDodge => "LinearDodge",
            BlendMode::LinearBurn => "LinearBurn",
            BlendMode::HardLight => "HardLight",
            BlendMode::SoftLight => "SoftLight",
            BlendMode::VividLight => "VividLight",
            BlendMode::LinearLight => "LinearLight",
            BlendMode::PinLight => "PinLight",
            BlendMode::HardMix => "HardMix",
            BlendMode::Difference => "Difference",
            BlendMode::Exclusion => "Exclusion",
            BlendMode::Subtract => "Subtract",
            BlendMode::Divide => "Divide",
            BlendMode::Hue => "Hue",
            BlendMode::Saturation => "Saturation",
            BlendMode::Color => "Color",
            BlendMode::Luminosity => "Luminosity",
            BlendMode::DarkerColor => "DarkerColor",
            BlendMode::LighterColor => "LighterColor",
            BlendMode::PassThrough => "PassThrough",
        }
    }

    /// 名前から（C# の列挙の名前と同じ綴り）。
    pub fn from_name(name: &str) -> Option<BlendMode> {
        Self::LAYER_MODES
            .iter()
            .copied()
            .chain(std::iter::once(BlendMode::PassThrough))
            .find(|m| m.name() == name)
    }

    /// 2 つの値だけで決まる（成分ごとの）モード。Multiply 〜 Divide。
    pub fn is_separable(self) -> bool {
        (BlendMode::Multiply as u8..=BlendMode::Divide as u8).contains(&(self as u8))
    }
}

/// チャンネル（Substance Painter のチャンネル）の ID。文書のチャンネルの一覧（[`ChannelInfo`]）の番号で、0〜5 は標準の 6 つ
/// （番号は Unity 版の PaintChannel と同じで保存形式に入る）、6 以上はユーザーチャンネル（文書が番号を配る）。
///
/// 標準のチャンネルは列挙のときと同じ名前の定数（`Channel::Color` など）で、`match` の型にも使える。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default, PartialOrd, Ord)]
pub struct Channel(u8);

#[allow(non_upper_case_globals)]
impl Channel {
    pub const Color: Channel = Channel(0);
    pub const Roughness: Channel = Channel(1);
    pub const Metallic: Channel = Channel(2);
    pub const Height: Channel = Channel(3);
    /// 接空間の単位ベクトル（OpenGL、Y+）。合成の式が色と違う（C# の NormalMaps）。
    pub const Normal: Channel = Channel(4);
    pub const Emission: Channel = Channel(5);

    /// 標準の 6 つ（番号の順）。
    pub const ALL: [Channel; 6] = [
        Channel::Color,
        Channel::Roughness,
        Channel::Metallic,
        Channel::Height,
        Channel::Normal,
        Channel::Emission,
    ];
    /// 標準のチャンネルの数（ユーザーチャンネルはこの番号から）。
    pub const STANDARD_COUNT: usize = 6;
    /// 1 つの文書が持てるチャンネルの数（標準を含む）。レイヤーは有効なチャンネルを 64 bit の印で持つ。
    pub const MAX: usize = 64;

    /// 番号から（0〜63。文書にあるかは [`crate::Document::channel_info`] で確かめる）。
    pub const fn from_index(index: usize) -> Option<Channel> {
        if index < Self::MAX {
            Some(Channel(index as u8))
        } else {
            None
        }
    }
    /// 番号（保存形式・配列の添え字）。
    #[inline]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
    /// 標準の 6 つのどれか。
    pub const fn is_standard(self) -> bool {
        (self.0 as usize) < Self::STANDARD_COUNT
    }
    /// 有効なチャンネルの印のビット。
    #[inline]
    pub(crate) const fn bit(self) -> u64 {
        1u64 << self.0
    }
    /// 標準のチャンネルの名前（C# の列挙の名前と同じ綴り）。ユーザーチャンネルは None。
    pub fn standard_name(self) -> Option<&'static str> {
        Some(match self.0 {
            0 => "Color",
            1 => "Roughness",
            2 => "Metallic",
            3 => "Height",
            4 => "Normal",
            5 => "Emission",
            _ => return None,
        })
    }
    /// 標準のチャンネルを名前から。
    pub fn from_standard_name(name: &str) -> Option<Channel> {
        Self::ALL
            .iter()
            .copied()
            .find(|c| c.standard_name() == Some(name))
    }
}

impl fmt::Debug for Channel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.standard_name() {
            Some(name) => f.write_str(name),
            None => write!(f, "User({})", self.0),
        }
    }
}

/// チャンネルの値の種類。合成の式と、調整・色の変化が使えるかを決める。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ChannelKind {
    /// 色（RGB に意味がある。色相・彩度の調整や色の変化が使える）。Color・Emission。
    Color,
    /// スカラー（R に値。灰色で描く）。Roughness・Metallic・Height。
    Scalar,
    /// 接空間の法線（単位ベクトルとして合成する）。Normal。
    Normal,
}

/// チャンネルの値の色空間（書き出しと Unity の取り込みの sRGB の印）。合成は保存したままの値で行い、変換しない。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ColorSpace {
    Srgb,
    Linear,
}

/// 文書のチャンネルの一覧の 1 つ。標準の 6 つは [`ChannelInfo::standard`]、ユーザーチャンネルは呼び手が決める。
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ChannelInfo {
    /// 表示の名前（空でない、128 文字まで）。
    pub name: String,
    pub kind: ChannelKind,
    pub color_space: ColorSpace,
    /// 何も描いていない所の値（書き出しやマテリアルの既定に使う。合成は透明から始める）。
    pub default: Rgba8,
}

impl ChannelInfo {
    /// 標準のチャンネルの情報。既定の値は Unity 版の書き出しの既定に合わせる（Roughness は Standard の平滑度 0.5、法線は平ら）。
    pub fn standard(channel: Channel) -> Option<ChannelInfo> {
        let (kind, space, default) = match channel {
            Channel::Color => (
                ChannelKind::Color,
                ColorSpace::Srgb,
                Rgba8::new(255, 255, 255, 255),
            ),
            Channel::Roughness => (
                ChannelKind::Scalar,
                ColorSpace::Linear,
                Rgba8::new(128, 128, 128, 255),
            ),
            Channel::Metallic => (
                ChannelKind::Scalar,
                ColorSpace::Linear,
                Rgba8::new(0, 0, 0, 255),
            ),
            Channel::Height => (
                ChannelKind::Scalar,
                ColorSpace::Linear,
                Rgba8::new(0, 0, 0, 255),
            ),
            Channel::Normal => (
                ChannelKind::Normal,
                ColorSpace::Linear,
                Rgba8::new(128, 128, 255, 255),
            ),
            Channel::Emission => (
                ChannelKind::Color,
                ColorSpace::Srgb,
                Rgba8::new(0, 0, 0, 255),
            ),
            _ => return None,
        };
        Some(ChannelInfo {
            name: channel.standard_name()?.to_string(),
            kind,
            color_space: space,
            default,
        })
    }
    /// 名前の検査（空でない・128 文字まで・制御文字を含まない）。
    pub(crate) fn validate(&self) -> Result<(), crate::CoreError> {
        let chars = self.name.chars().count();
        if chars == 0 || chars > 128 || self.name.chars().any(char::is_control) {
            return Err(crate::CoreError::InvalidArgument("チャンネルの名前"));
        }
        Ok(())
    }
}

/// レイヤーの種類（値は保存形式に入る。C# の LayerKind と同じ）。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
#[repr(u8)]
pub enum LayerKind {
    /// 画素を持つレイヤー（チャンネルごとの面）。
    #[default]
    Raster = 0,
    /// チャンネルごとの 1 つの値でキャンバス全体を埋めるレイヤー（画素は持たない）。
    Fill = 1,
    /// 下の合成を変えるレイヤー（反転・レベル補正・色相/彩度/明度）。
    Adjustment = 2,
    /// グループ（中身は直下に並ぶレイヤー）。
    Group = 3,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 列挙だったときの書き方（定数の型の match・matches!・既定値・並び）がそのまま使える。
    #[test]
    fn channel_constants_work_like_the_old_enum() {
        assert!(matches!(
            Channel::Emission,
            Channel::Color | Channel::Emission
        ));
        let name = match Channel::Height {
            Channel::Color => "c",
            Channel::Height => "h",
            _ => "other",
        };
        assert_eq!(name, "h");
        assert_eq!(Channel::default(), Channel::Color);
        assert!(Channel::Color < Channel::Emission);
        assert_eq!(Channel::from_index(64), None);
        assert!(Channel::from_index(6).is_some_and(|c| !c.is_standard()));
    }
}
