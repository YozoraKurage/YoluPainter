//! 文書（タイル・レイヤー・チャンネル）と CPU の合成・ブラシ・フィルター・Generator、3D の面の計算（`geometry`）、スキンとポーズ（`skin`）。GPU にも OS にも頼らない純粋な計算で、cargo test で速く確かめる。
//!
//! 式・丸め・straight RGBA8 は、もと Unity 版の C# の Core（YoluPainter の `Runtime/Core`）を仕様にし、C# の Core に同じ入力を
//! 通した出力（`tests/golden/`、`tools/csharp-golden/` で作る）とのバイト一致で確かめてきた。レイヤーの合成の式（[`blend`]）は、
//! この crate の f32 の式が正本で、合成を通る正解はこの crate で撮り直した（`YOLU_GOLDEN_UPDATE=1`）。Normal のチャンネルの
//! 合成（[`normal`]）とブラシの画素（[`brush`]）も同じ f32 のレーンの式。フィルター・Generator の値など、f64 のままの式は今も C# の
//! 正解と同じ値。
//!
//! 座標は左下原点（画素 (0, 0) が左下、その中心は (0.5, 0.5)）。画素の並びは行優先で、一番下の行が先。
//!
//! レイヤーの種類（[`LayerKind`]）はラスター・塗りつぶし（チャンネルごとの値）・調整（反転・レベル補正・色相/彩度/明度と、Rust 版だけのグラデーションマップ・トーンカーブ・カラーバランス・明るさ/コントラスト・2 値化・
//! ポスタリゼーション。`adjust`）・グループ
//! （通過・分離、入れ子）。どのレイヤーもラスターマスク（[`RasterMask`]、全チャンネルで共有）とクリッピングを持てる。チャンネル
//! （[`Channel`]）は文書の一覧の番号で、0〜5 は Unity 版と同じ標準の 6 つ、ユーザーチャンネルは [`Document::add_channel`] で足す。
//! レイヤーはチャンネルごとに有効と合成モード・不透明度（[`ChannelBlend`]）を持つ。Normal の種類のチャンネルは単位ベクトルとして
//! 合成し（[`normal`]）、文書の Normal の出力は Height から作った法線を土台にできる（[`NormalSettings`]）。
//!
//! ```
//! use yolu_core::{Document, BrushSettings, Rgba8, glam::DVec2};
//! let mut doc = Document::new(256, 256).unwrap();
//! let layer = doc.add_layer("レイヤー 1").unwrap();
//! let brush = BrushSettings { color: Rgba8::new(200, 30, 30, 255), ..BrushSettings::default() };
//! let mut stroke = doc.begin_stroke(layer, &brush).unwrap();
//! stroke.add_point(&mut doc, 20.0, 20.0, 0.5, DVec2::ZERO).unwrap();
//! stroke.add_point(&mut doc, 200.0, 120.0, 1.0, DVec2::ZERO).unwrap();
//! assert!(doc.end_stroke(stroke).unwrap().changed);
//! let pixels = doc.composite(doc.bounds()).unwrap();
//! assert_eq!(pixels.len(), 256 * 256 * 4);
//! doc.undo().unwrap();
//!
//! // グループとマスク
//! let a = doc.add_layer("a").unwrap();
//! let group = doc.group_layers(&[a], "グループ").unwrap();
//! doc.set_layer_blend_mode(group, yolu_core::BlendMode::Multiply).unwrap(); // 分離
//! doc.add_layer_mask(group).unwrap();
//! doc.set_mask_pixel(group, 10, 10, 255).unwrap(); // (10, 10) を隠す
//! ```

// 画素（4 バイト）を chunks_exact(4) で回すのは読みやすさのため。0〜1 への切り詰めは C# と同じ比較の順で書く（f64::clamp にしない）。
#![allow(clippy::chunks_exact_to_as_chunks, clippy::manual_clamp)]

mod adjust;
pub mod blend;
pub mod brush;
mod composite;
pub mod curve;
mod document;
pub mod effects;
mod error;
pub mod export;
pub mod fill_image;
pub mod fill_points;
pub mod filter;
pub mod generator;
pub mod geometry;
pub mod id_colors;
mod layer;
pub mod look;
pub mod material;
pub mod material_triangles;
mod math;
pub mod mesh_maps;
pub mod normal;
pub mod padding;
pub mod paths;
mod ranges;
pub mod rulers;
pub mod selection;
pub mod skin;
pub mod smart;
pub mod smart_library;
mod surface;
mod symmetry;
pub mod text;
pub mod tile_cache;
mod types;
pub mod uv_layout;

pub use adjust::{
    luminance, AdjustmentSettings, AdjustmentType, BalanceRange, BrightnessContrast, ColorAdjust,
    ColorBalance, GradientMap, Posterize, Threshold, ToneChannel, ToneCurves,
};
pub use brush::{
    builtin_presets, builtin_tip, AntiAlias, Brush, BrushEffect, BrushMappedPixel, BrushPixel,
    BrushPreset, BrushSample, BrushSettings, BrushSourceTap, BrushStencil, BrushTip, ColorDynamics,
    ColorMix, Controls, DualBrush, DualBrushMode, ImageColorSpace, Jitter, MixGround, MixMode,
    PaperTexture, PressureResponse, PressureResponses, StencilImage, StencilMapping, StencilMode,
    StencilPoint, StencilTiling, StrokeAssist, TextureMode, TipSelection, TipShape,
};
pub use document::{
    clean_saved_name, Affine2D, CanvasResampling, ClipboardRefusal, ClipboardSource,
    CompositedTile, Document, EffectCounters, Homography, LayerLocks, LayerMergeReport, LiquifyDab,
    LiquifyMode, MergeMethod, MergeRefusal, PasteResult, PixelClipboard, PreparedResize,
    Resampling, ResizeReport, SavedSelection, SeamFallback, Stroke, StrokeResult, StrokeStats,
    TriangleFill, Warp, WarpMesh, WarpPoint, DEFAULT_SOURCE_BUDGET_BYTES, MAX_GROUP_DEPTH,
    MAX_SAVED_NAME_CHARS, MAX_SAVED_SELECTIONS,
};
pub use effects::{
    Anchor, AnchorId, AnchorInfo, AnchorIssue, AnchorIssueKind, AnchorPlacement, EffectInputs,
    EffectSettings, FallbackEffect, FilterEffect, FilterId, FilterSpec, FilterTarget, ImageId,
    ImageInput, InactiveEffect, InactiveReason, InactiveTarget, LayerPath, MapInput, ModelFrame,
};
pub use error::CoreError;
pub use glam;
pub use layer::{ChannelBlend, Layer, LayerId, RasterMask};
pub use normal::{HeightEdgeMode, NormalSettings, NormalYDirection};
pub use paths::LayerPathEntry;
pub use rulers::{
    Ruler, RulerId, RulerKind, RulerPlace, RulerRef, RulerScope, RulerSpace, SpecialRulers,
    MAX_RULERS_PER_LAYER,
};
pub use selection::{SelectionCombine, SelectionMask};
pub use surface::Surface;
pub use symmetry::{CanvasSymmetry, SymmetryMode, SymmetryTransform};
pub use types::{
    BlendMode, Channel, ChannelInfo, ChannelKind, ColorSpace, LayerKind, Rect, Rgba8, RowOrder,
    TileCoord,
};

pub use composite::MemoStats;
pub use document::HistoryKind;
