//! core の口（yolu-core）。画面の側は `engine::*` だけを使い、core の形との違いはここで合わせる。
//!
//! core は左下原点・straight RGBA8・下の行が先。ストロークは文書を借りない札（`Stroke`）で、点は `stroke.add_point(&mut doc, …)`。
//! 変わったタイルは `changed_tiles(channel, since)` と `change_serial()` で、読む側が通し番号を覚える（読んでも消えない）。

pub use yolu_core::glam::DVec2;
pub use yolu_core::selection::{DEFAULT_WORKING_BUDGET_BYTES, MAX_MODIFY_RADIUS};
pub use yolu_core::DEFAULT_SOURCE_BUDGET_BYTES;
pub use yolu_core::{
    AdjustmentSettings, AdjustmentType, BalanceRange, BlendMode, BrightnessContrast, Brush,
    BrushEffect, BrushPreset, BrushSample, BrushSettings, BrushTip, CanvasResampling,
    CanvasSymmetry, Channel, ChannelBlend, ChannelInfo, ChannelKind, ClipboardRefusal,
    ClipboardSource, ColorAdjust, ColorBalance, ColorDynamics, ColorSpace, CompositedTile,
    Controls, CoreError, Document, DualBrush, DualBrushMode, GradientMap, HeightEdgeMode, Jitter,
    Layer, LayerId, LayerKind, NormalSettings, NormalYDirection, PaperTexture, PixelClipboard,
    Posterize, PreparedResize, PressureResponse, PressureResponses, Rect, Rgba8, RowOrder,
    SelectionCombine, SelectionMask, Stroke, StrokeAssist, SymmetryMode, TextureMode, Threshold,
    TileCoord, TipShape, ToneChannel, ToneCurves,
};
pub use yolu_core::{AntiAlias, ColorMix, MixGround, MixMode};

/// ペンの傾き（度。Windows の POINTER_PEN_INFO の tiltX・tiltY と同じく −90〜90）。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Tilt {
    pub x: f32,
    pub y: f32,
}

impl Tilt {
    /// core に渡す形（ラジアン）。
    pub fn radians(self) -> DVec2 {
        DVec2::new((self.x as f64).to_radians(), (self.y as f64).to_radians())
    }
}

/// 合成の 1 画素（straight RGBA8。キャンバスの外は透明）。
pub fn composite_pixel(doc: &Document, x: u32, y: u32) -> [u8; 4] {
    doc.composite_pixel(Channel::Color, x, y)
        .map(|p| [p.r, p.g, p.b, p.a])
        .unwrap_or([0; 4])
}

/// レイヤーの Color の 1 画素（straight RGBA8。無ければ透明）。サムネイル用。
pub fn layer_pixel(layer: &Layer, x: u32, y: u32) -> [u8; 4] {
    layer
        .surface(Channel::Color)
        .and_then(|s| s.pixel(x, y).ok())
        .map(|p| [p.r, p.g, p.b, p.a])
        .unwrap_or([0; 4])
}

/// レイヤーが Color の画素を持つか。
pub fn layer_has_pixels(layer: &Layer) -> bool {
    layer
        .surface(Channel::Color)
        .is_some_and(|s| s.tile_count() > 0)
}

pub use yolu_core::HistoryKind;
