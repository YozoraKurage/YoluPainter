//! 文書（タイル・レイヤー・チャンネル）と CPU の合成・ブラシ・フィルター・Generator。GPU にも OS にも頼らない純粋な計算で、cargo test で速く確かめる。
//!
//! 仕様は Unity 版の C# の Core（YoluPainter の `Runtime/Core`）。式・丸め・straight RGBA8 は同じにし、C# の Core に同じ入力を
//! 通した出力（`tests/golden/`、`tools/csharp-golden/` で作る）とバイト一致を確かめる。
//!
//! 座標は左下原点（画素 (0, 0) が左下、その中心は (0.5, 0.5)）。画素の並びは行優先で、一番下の行が先。
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
//! ```

// 画素（4 バイト）を chunks_exact(4) で回すのは読みやすさのため。0〜1 への切り詰めは C# と同じ比較の順で書く（f64::clamp にしない）。
#![allow(clippy::chunks_exact_to_as_chunks, clippy::manual_clamp)]

pub mod blend;
mod brush;
mod composite;
mod document;
mod error;
mod math;
mod surface;
mod types;

pub use brush::{BrushSample, BrushSettings};
pub use document::{Document, Layer, LayerId, Stroke, StrokeResult, StrokeStats};
pub use error::CoreError;
pub use glam;
pub use surface::Surface;
pub use types::{BlendMode, Channel, Rect, Rgba8, RowOrder, TileCoord};
