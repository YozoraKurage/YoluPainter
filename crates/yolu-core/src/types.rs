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
    /// グループだけ: 中身をグループでないかのように下へ重ねる。M1 にはグループが無いので、レイヤーには付けられない。
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

/// レイヤーの持つチャンネル（Substance Painter のチャンネル）。M1 で描けるのは Color だけだが、面はチャンネルごとに持つ形にしてある。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default, PartialOrd, Ord)]
#[repr(u8)]
pub enum Channel {
    #[default]
    Color = 0,
    Roughness,
    Metallic,
    Height,
    /// 接空間の単位ベクトル。合成の式が色と違う（C# の NormalMaps）ので、M1 では合成しない。
    Normal,
    Emission,
}

impl Channel {
    pub const ALL: [Channel; 6] = [
        Channel::Color,
        Channel::Roughness,
        Channel::Metallic,
        Channel::Height,
        Channel::Normal,
        Channel::Emission,
    ];
}
